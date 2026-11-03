//! Websocket for presence, chat events, and WebRTC signaling.
//!
//! Media is UDP. This socket carries JSON signals only.

use std::io::ErrorKind;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use banter_server::{ClientSignal, ServerEvent};

use crate::profiler;

pub struct Realtime {
    signals: Sender<ClientSignal>,
    events: Arc<Mutex<Vec<ServerEvent>>>,
    error: Arc<Mutex<Option<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Realtime {
    pub fn connect(url: String) -> Self {
        let (signal_tx, signal_rx) = mpsc::channel();
        let events = Arc::new(Mutex::new(Vec::new()));
        let error = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = thread::spawn({
            let events = Arc::clone(&events);
            let error = Arc::clone(&error);
            let stop = Arc::clone(&stop);
            move || run(url, signal_rx, events, error, stop)
        });
        Self {
            signals: signal_tx,
            events,
            error,
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
}

impl Drop for Realtime {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run(
    url: String,
    signal_rx: Receiver<ClientSignal>,
    events: Arc<Mutex<Vec<ServerEvent>>>,
    error: Arc<Mutex<Option<String>>>,
    stop: Arc<AtomicBool>,
) {
    let mut socket = match tungstenite::connect(&url) {
        Ok((socket, _)) => socket,
        Err(err) => {
            *error.lock().expect("error lock") = Some(format!("realtime connection failed: {err}"));
            return;
        }
    };
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
            Ok(tungstenite::Message::Close(_)) => break,
            Ok(_) => {}
            Err(tungstenite::Error::Io(err))
                if err.kind() == ErrorKind::TimedOut || err.kind() == ErrorKind::WouldBlock => {}
            Err(tungstenite::Error::ConnectionClosed) => break,
            Err(err) => {
                *error.lock().expect("error lock") = Some(format!("realtime connection dropped: {err}"));
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
                    if socket.send(tungstenite::Message::Text(text.into())).is_err() {
                        return;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
    }
}
