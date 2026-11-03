//! Websocket for presence, chat events, and media frames.

use std::io::ErrorKind;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use banter_server::{MediaFrame, ServerEvent, decode_media_frame};

pub struct Realtime {
    binary: Sender<Vec<u8>>,
    activity: Sender<()>,
    events: Arc<Mutex<Vec<ServerEvent>>>,
    frames: Arc<Mutex<Vec<MediaFrame>>>,
    error: Arc<Mutex<Option<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Realtime {
    pub fn connect(url: String) -> Self {
        let (binary_tx, binary_rx) = mpsc::channel();
        let (activity_tx, activity_rx) = mpsc::channel();
        let events = Arc::new(Mutex::new(Vec::new()));
        let frames = Arc::new(Mutex::new(Vec::new()));
        let error = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = thread::spawn({
            let events = Arc::clone(&events);
            let frames = Arc::clone(&frames);
            let error = Arc::clone(&error);
            let stop = Arc::clone(&stop);
            move || run(url, binary_rx, activity_rx, events, frames, error, stop)
        });
        Self {
            binary: binary_tx,
            activity: activity_tx,
            events,
            frames,
            error,
            stop,
            thread: Some(thread),
        }
    }

    pub fn binary_sender(&self) -> Sender<Vec<u8>> {
        self.binary.clone()
    }

    pub fn poke_activity(&self) {
        let _ = self.activity.send(());
    }

    pub fn take_events(&self) -> Vec<ServerEvent> {
        std::mem::take(&mut *self.events.lock().expect("event lock"))
    }

    pub fn take_frames(&self) -> Vec<MediaFrame> {
        std::mem::take(&mut *self.frames.lock().expect("frame lock"))
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
    binary_rx: Receiver<Vec<u8>>,
    activity_rx: Receiver<()>,
    events: Arc<Mutex<Vec<ServerEvent>>>,
    frames: Arc<Mutex<Vec<MediaFrame>>>,
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
                if let Ok(event) = serde_json::from_str::<ServerEvent>(text.as_ref()) {
                    events.lock().expect("event lock").push(event);
                }
            }
            Ok(tungstenite::Message::Binary(bytes)) => {
                if let Ok(frame) = decode_media_frame(bytes.as_ref()) {
                    frames.lock().expect("frame lock").push(frame);
                }
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
            match binary_rx.try_recv() {
                Ok(bytes) => {
                    if socket.send(tungstenite::Message::Binary(bytes.into())).is_err() {
                        return;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        if activity_rx.try_recv().is_ok() {
            let _ = socket.send(tungstenite::Message::Text("activity".into()));
            while activity_rx.try_recv().is_ok() {}
        }
    }
}
