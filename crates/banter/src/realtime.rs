//! Websocket for presence, chat events, and WebRTC signaling.
//!
//! Media is UDP. This socket carries JSON signals only.

use std::io::ErrorKind;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use banter_server::{
    format_banter_notice, ClientSignal, ServerEvent, ERR_WS_CONNECT, ERR_WS_DROPPED,
};

use crate::profiler;

pub struct Realtime {
    signals: Sender<ClientSignal>,
    events: Arc<Mutex<Vec<ServerEvent>>>,
    error: Arc<Mutex<Option<String>>>,
    /// Set once the TCP/websocket handshake succeeds; cleared when the socket ends.
    connected: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Realtime {
    pub fn connect(url: String) -> Self {
        let (signal_tx, signal_rx) = mpsc::channel();
        let events = Arc::new(Mutex::new(Vec::new()));
        let error = Arc::new(Mutex::new(None));
        let connected = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = thread::spawn({
            let events = Arc::clone(&events);
            let error = Arc::clone(&error);
            let connected = Arc::clone(&connected);
            let stop = Arc::clone(&stop);
            move || run(url, signal_rx, events, error, connected, stop)
        });
        Self {
            signals: signal_tx,
            events,
            error,
            connected,
            stop,
            thread: Some(thread),
        }
    }

    pub fn signal_sender(&self) -> Sender<ClientSignal> {
        self.signals.clone()
    }

    pub fn send(&self, signal: ClientSignal) {
        let _ = self.signals.send(signal);
    }

    pub fn poke_activity(&self) {
        let _ = self.signals.send(ClientSignal::Activity);
    }

    pub fn take_events(&self) -> Vec<ServerEvent> {
        std::mem::take(&mut *self.events.lock().expect("event lock"))
    }

    pub fn take_error(&self) -> Option<String> {
        self.error.lock().expect("error lock").take()
    }

    /// True after the websocket handshake succeeds and until the socket drops.
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }
}

impl Drop for Realtime {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn mark_dropped(error: &Mutex<Option<String>>, connected: &AtomicBool, stop: &AtomicBool) {
    connected.store(false, Ordering::Relaxed);
    if stop.load(Ordering::Relaxed) {
        return;
    }
    let mut slot = error.lock().expect("error lock");
    if slot.is_none() {
        *slot = Some(format_banter_notice(
            ERR_WS_DROPPED,
            "realtime connection dropped",
        ));
    }
}

fn run(
    url: String,
    signal_rx: Receiver<ClientSignal>,
    events: Arc<Mutex<Vec<ServerEvent>>>,
    error: Arc<Mutex<Option<String>>>,
    connected: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
) {
    let mut socket = match tungstenite::connect(&url) {
        Ok((socket, _)) => socket,
        Err(err) => {
            *error.lock().expect("error lock") = Some(format_banter_notice(
                ERR_WS_CONNECT,
                "couldn't connect realtime",
            ));
            let _ = err;
            return;
        }
    };
    connected.store(true, Ordering::Relaxed);
    if let tungstenite::stream::MaybeTlsStream::Plain(tcp) = socket.get_mut() {
        let _ = tcp.set_read_timeout(Some(Duration::from_millis(40)));
        let _ = tcp.set_nodelay(true);
    }
    while !stop.load(Ordering::Relaxed) {
        match socket.read() {
            Ok(tungstenite::Message::Text(text)) => {
                profiler::record_ws_rx(text.len());
                if let Ok(event) = serde_json::from_str::<ServerEvent>(text.as_ref()) {
                    events.lock().expect("event lock").push(event);
                }
            }
            Ok(tungstenite::Message::Binary(bytes)) => {
                profiler::record_ws_rx(bytes.len());
            }
            Ok(tungstenite::Message::Ping(payload)) => {
                let _ = socket.send(tungstenite::Message::Pong(payload));
            }
            Ok(tungstenite::Message::Close(_)) => {
                mark_dropped(&error, &connected, &stop);
                break;
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(err))
                if err.kind() == ErrorKind::TimedOut || err.kind() == ErrorKind::WouldBlock => {}
            Err(tungstenite::Error::ConnectionClosed) => {
                mark_dropped(&error, &connected, &stop);
                break;
            }
            Err(err) => {
                mark_dropped(&error, &connected, &stop);
                let _ = err;
                break;
            }
        }
        loop {
            match signal_rx.try_recv() {
                Ok(signal) => {
                    let text = match serde_json::to_string(&signal) {
                        Ok(text) => text,
                        Err(_) => continue,
                    };
                    profiler::record_ws_tx(text.len());
                    if socket
                        .send(tungstenite::Message::Text(text.into()))
                        .is_err()
                    {
                        mark_dropped(&error, &connected, &stop);
                        return;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    mark_dropped(&error, &connected, &stop);
                    return;
                }
            }
        }
    }
    connected.store(false, Ordering::Relaxed);
}
