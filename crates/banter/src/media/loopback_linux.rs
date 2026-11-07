//! Linux desktop and per-app capture.
//!
//! Speaker loopback uses the default PulseAudio sink monitor (`parec`).
//! Per-app capture uses the PipeWire node whose `application.process.id`
//! matches the window, recorded with `pw-cat`. PCM is 48 kHz mono i16 in the
//! same `loop:` and `app:` slots Windows fills. A missing device is an error,
//! not silence.

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub struct Loop {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Loop {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn start_render(
    id: String,
    name: String,
    pcm: Arc<Mutex<HashMap<String, VecDeque<i16>>>>,
) -> Result<Loop, String> {
    let device = monitor_source(&name)?;
    let mut child = Command::new("parec")
        .args([
            "-d",
            &device,
            "--raw",
            "--format=s16le",
            "--rate=48000",
            "--channels=1",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| "speaker loopback needs PulseAudio (`parec` was not found)".to_string())?;
    settle(&mut child, "speaker loopback")?;
    spawn_reader(child, id, pcm)
}

pub fn start_process(
    id: String,
    pid: u32,
    pcm: Arc<Mutex<HashMap<String, VecDeque<i16>>>>,
) -> Result<Loop, String> {
    let node = pipewire_node_for_pid(pid)?;
    let mut child = Command::new("pw-cat")
        .args([
            "--record",
            "--target",
            &node,
            "--format",
            "s16",
            "--rate",
            "48000",
            "--channels",
            "1",
            "-",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| "app audio needs PipeWire (`pw-cat` was not found)".to_string())?;
    settle(&mut child, "app audio")?;
    spawn_reader(child, id, pcm)
}

fn spawn_reader(
    mut child: Child,
    id: String,
    pcm: Arc<Mutex<HashMap<String, VecDeque<i16>>>>,
) -> Result<Loop, String> {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = Arc::clone(&stop);
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "capture produced no audio".to_string())?;
    let thread = thread::Builder::new()
        .name("banter-loopback".into())
        .spawn(move || {
            let mut stdout = stdout;
            let mut bytes = [0u8; 960 * 2];
            while !stop_thread.load(Ordering::Relaxed) {
                match stdout.read(&mut bytes) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => push_s16(&id, &bytes[..n - n % 2], &pcm),
                }
            }
            let _ = child.kill();
            let _ = child.wait();
        })
        .map_err(|err| err.to_string())?;
    Ok(Loop {
        stop,
        thread: Some(thread),
    })
}

fn settle(child: &mut Child, what: &str) -> Result<(), String> {
    thread::sleep(Duration::from_millis(120));
    if let Some(status) = child.try_wait().map_err(|err| err.to_string())? {
        let mut detail = String::new();
        if let Some(stderr) = child.stderr.as_mut() {
            let mut reader = BufReader::new(stderr);
            let _ = reader.read_line(&mut detail);
        }
        let detail = detail.trim();
        if detail.is_empty() {
            return Err(format!("{what} exited ({status})"));
        }
        return Err(format!("{what}: {detail}"));
    }
    Ok(())
}

fn push_s16(id: &str, bytes: &[u8], pcm: &Mutex<HashMap<String, VecDeque<i16>>>) {
    let mut map = pcm.lock().expect("source audio");
    let bucket = map.entry(id.to_string()).or_default();
    for sample in bytes.chunks_exact(2) {
        bucket.push_back(i16::from_le_bytes([sample[0], sample[1]]));
    }
    while bucket.len() > super::pcm_backlog_samples() {
        bucket.pop_front();
    }
}

fn monitor_source(name: &str) -> Result<String, String> {
    let output = Command::new("pactl")
        .args(["list", "short", "sources"])
        .output()
        .map_err(|_| "speaker loopback needs PulseAudio (`pactl` was not found)".to_string())?;
    if !output.status.success() {
        return Err("speaker loopback could not list PulseAudio sources".into());
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let monitors: Vec<&str> = text
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .filter(|source| source.ends_with(".monitor"))
        .collect();
    if monitors.is_empty() {
        return Err("no PulseAudio sink monitor is available".into());
    }
    let needle = name.trim().to_ascii_lowercase();
    if !needle.is_empty() {
        if let Some(found) = monitors
            .iter()
            .find(|source| source.to_ascii_lowercase().contains(&needle))
        {
            return Ok((*found).to_string());
        }
    }
    let default_sink = Command::new("pactl")
        .args(["get-default-sink"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .unwrap_or_default();
    if !default_sink.is_empty() {
        let monitor = format!("{default_sink}.monitor");
        if monitors.iter().any(|source| *source == monitor) {
            return Ok(monitor);
        }
    }
    Ok(monitors[0].to_string())
}

fn pipewire_node_for_pid(pid: u32) -> Result<String, String> {
    let output = Command::new("pw-dump")
        .output()
        .map_err(|_| "app audio needs PipeWire (`pw-dump` was not found)".to_string())?;
    if !output.status.success() {
        return Err("PipeWire did not list audio nodes".into());
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let needle = format!("\"application.process.id\": {pid}");
    let mut node_id: Option<String> = None;
    let mut current_id: Option<String> = None;
    let mut matched = false;
    let mut output_stream = false;
    for line in text.lines() {
        let trimmed = line.trim().trim_end_matches(',');
        if trimmed == "{" {
            if matched && output_stream {
                if let Some(id) = current_id.clone() {
                    return Ok(id);
                }
            }
            current_id = None;
            matched = false;
            output_stream = false;
        }
        if let Some(rest) = trimmed.strip_prefix("\"id\": ") {
            current_id = Some(rest.trim().to_string());
        }
        if trimmed.contains(&needle) {
            matched = true;
        }
        if trimmed.contains("Stream/Output/Audio") {
            output_stream = true;
        }
        if matched && output_stream {
            node_id = current_id.clone();
        }
    }
    node_id.ok_or_else(|| format!("no PipeWire output stream for process {pid}"))
}
