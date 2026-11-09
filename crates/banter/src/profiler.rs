//! Client performance counters and a ring of recent expensive operations.
//!
//! Process-wide so HTTP, WebSocket, and media threads can record without UI handles.
//! Toggle the panel with F3.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const OPS_CAP: usize = 256;
const DELTA_CAP: usize = 120;
const OP_THRESHOLD_MS: f32 = 1.0;
const LARGE_BODY: usize = 8 * 1024;

#[derive(Clone, Copy, Debug)]
pub enum Section {
    Prepare,
    Draw,
    Gl,
}

#[derive(Clone, Debug)]
pub struct OpRecord {
    pub label: String,
    pub duration_ms: f32,
    pub detail: String,
}

#[derive(Clone, Debug)]
pub struct QosView {
    pub topology: String,
    pub ladder: String,
    pub subscribers: u32,
    pub layers: String,
    pub backend: String,
    pub loss: f32,
    pub rtt_ms: u32,
    pub jitter_ms: u32,
    pub user_kbps: u32,
    pub stream_kbps: u32,
    pub camera_kbps: u32,
    pub screen_kbps: u32,
    pub keyframes: u32,
    pub encode_ms: f32,
    pub decode_ms: f32,
    pub step: String,
    /// Local Go Live / camera publish snapshot for `/debugnow`.
    pub publishing_screen: bool,
    pub publishing_camera: bool,
    pub target_kbps: u32,
    pub encode_width: u32,
    pub encode_height: u32,
    pub encode_fps: u32,
    pub encoder_pref: String,
    pub rate_control: String,
    pub keyframe_secs: u32,
    /// Latest remote video tile summary (empty when nobody is sending video).
    pub remote_stream: String,
    /// Screen capture pipeline line (FPS / drops / stage p95). Empty when idle.
    pub capture_path: String,
}

impl Default for QosView {
    fn default() -> Self {
        Self {
            topology: "idle".into(),
            ladder: "nominal".into(),
            subscribers: 0,
            layers: "all".into(),
            backend: "openh264".into(),
            loss: 0.0,
            rtt_ms: 0,
            jitter_ms: 0,
            user_kbps: 0,
            stream_kbps: 0,
            camera_kbps: 0,
            screen_kbps: 0,
            keyframes: 0,
            encode_ms: 0.0,
            decode_ms: 0.0,
            step: "nominal".into(),
            publishing_screen: false,
            publishing_camera: false,
            target_kbps: 0,
            encode_width: 0,
            encode_height: 0,
            encode_fps: 0,
            encoder_pref: "auto".into(),
            rate_control: "cbr".into(),
            keyframe_secs: 1,
            remote_stream: String::new(),
            capture_path: String::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Snapshot {
    pub fps: f32,
    pub fps_smooth: f32,
    pub prepare_ms: f32,
    pub draw_ms: f32,
    pub gl_ms: f32,
    pub rss_bytes: u64,
    pub vmsize_bytes: u64,
    pub http_tx: u64,
    pub http_rx: u64,
    pub http_count: u64,
    pub http_tx_per_s: u64,
    pub http_rx_per_s: u64,
    pub http_count_per_s: u64,
    pub ws_tx: u64,
    pub ws_rx: u64,
    pub ws_msgs: u64,
    pub ws_tx_per_s: u64,
    pub ws_rx_per_s: u64,
    pub ws_msgs_per_s: u64,
}

struct RateMark {
    at: Instant,
    http_tx: u64,
    http_rx: u64,
    http_count: u64,
    ws_tx: u64,
    ws_rx: u64,
    ws_msgs: u64,
}

pub struct Profiler {
    open: AtomicBool,
    fps: AtomicU32,
    fps_smooth: AtomicU32,
    prepare_ms: AtomicU32,
    draw_ms: AtomicU32,
    gl_ms: AtomicU32,
    rss_bytes: AtomicU64,
    vmsize_bytes: AtomicU64,
    http_tx: AtomicU64,
    http_rx: AtomicU64,
    http_count: AtomicU64,
    ws_tx: AtomicU64,
    ws_rx: AtomicU64,
    ws_msgs: AtomicU64,
    http_tx_per_s: AtomicU64,
    http_rx_per_s: AtomicU64,
    http_count_per_s: AtomicU64,
    ws_tx_per_s: AtomicU64,
    ws_rx_per_s: AtomicU64,
    ws_msgs_per_s: AtomicU64,
    rates: Mutex<RateMark>,
    mem_at: Mutex<Instant>,
    deltas: Mutex<VecDeque<f32>>,
    ops: Mutex<VecDeque<OpRecord>>,
    qos: Mutex<QosView>,
}

impl Profiler {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            open: AtomicBool::new(false),
            fps: AtomicU32::new(0),
            fps_smooth: AtomicU32::new(0),
            prepare_ms: AtomicU32::new(0),
            draw_ms: AtomicU32::new(0),
            gl_ms: AtomicU32::new(0),
            rss_bytes: AtomicU64::new(0),
            vmsize_bytes: AtomicU64::new(0),
            http_tx: AtomicU64::new(0),
            http_rx: AtomicU64::new(0),
            http_count: AtomicU64::new(0),
            ws_tx: AtomicU64::new(0),
            ws_rx: AtomicU64::new(0),
            ws_msgs: AtomicU64::new(0),
            http_tx_per_s: AtomicU64::new(0),
            http_rx_per_s: AtomicU64::new(0),
            http_count_per_s: AtomicU64::new(0),
            ws_tx_per_s: AtomicU64::new(0),
            ws_rx_per_s: AtomicU64::new(0),
            ws_msgs_per_s: AtomicU64::new(0),
            rates: Mutex::new(RateMark {
                at: now,
                http_tx: 0,
                http_rx: 0,
                http_count: 0,
                ws_tx: 0,
                ws_rx: 0,
                ws_msgs: 0,
            }),
            mem_at: Mutex::new(now - Duration::from_secs(2)),
            deltas: Mutex::new(VecDeque::with_capacity(DELTA_CAP)),
            ops: Mutex::new(VecDeque::with_capacity(OPS_CAP)),
            qos: Mutex::new(QosView::default()),
        }
    }

    pub fn set_qos(&self, mut view: QosView) {
        let mut guard = self.qos.lock().expect("qos");
        view.decode_ms = guard.decode_ms;
        view.remote_stream = guard.remote_stream.clone();
        *guard = view;
    }

    pub fn note_decode(&self, ms: f32) {
        self.qos.lock().expect("qos").decode_ms = ms;
    }

    pub fn note_remote_stream(&self, summary: String) {
        self.qos.lock().expect("qos").remote_stream = summary;
    }

    pub fn qos(&self) -> QosView {
        self.qos.lock().expect("qos").clone()
    }

    pub fn is_open(&self) -> bool {
        self.open.load(Ordering::Relaxed)
    }

    pub fn set_open(&self, open: bool) {
        self.open.store(open, Ordering::Relaxed);
    }

    pub fn toggle(&self) {
        let _ = self.open.fetch_xor(true, Ordering::Relaxed);
    }

    pub fn frame(&self, delta: f64) {
        let delta = delta.max(1.0 / 1000.0) as f32;
        let instant = 1.0 / delta;
        store_f32(&self.fps, instant);
        let smooth = load_f32(&self.fps_smooth);
        let next = if smooth <= 0.0 {
            instant
        } else {
            smooth * 0.9 + instant * 0.1
        };
        store_f32(&self.fps_smooth, next);
        if let Ok(mut deltas) = self.deltas.lock() {
            if deltas.len() >= DELTA_CAP {
                deltas.pop_front();
            }
            deltas.push_back(delta);
        }
        self.refresh_rates();
        self.refresh_memory();
    }

    pub fn set_section(&self, section: Section, ms: f32) {
        let slot = match section {
            Section::Prepare => &self.prepare_ms,
            Section::Draw => &self.draw_ms,
            Section::Gl => &self.gl_ms,
        };
        store_f32(slot, ms);
    }

    pub fn record_op(&self, label: impl Into<String>, duration_ms: f32, detail: impl Into<String>) {
        if duration_ms < OP_THRESHOLD_MS {
            return;
        }
        let record = OpRecord {
            label: label.into(),
            duration_ms,
            detail: detail.into(),
        };
        if let Ok(mut ops) = self.ops.lock() {
            if ops.len() >= OPS_CAP {
                ops.pop_front();
            }
            ops.push_back(record);
        }
    }

    pub fn record_http(
        &self,
        method: &str,
        path: &str,
        elapsed: Duration,
        req_bytes: usize,
        resp_bytes: usize,
    ) {
        let ms = elapsed.as_secs_f32() * 1000.0;
        self.http_tx.fetch_add(req_bytes as u64, Ordering::Relaxed);
        self.http_rx.fetch_add(resp_bytes as u64, Ordering::Relaxed);
        self.http_count.fetch_add(1, Ordering::Relaxed);
        if ms >= OP_THRESHOLD_MS || req_bytes >= LARGE_BODY || resp_bytes >= LARGE_BODY {
            self.record_op(
                format!("HTTP {method} {path}"),
                ms,
                format!(
                    "{} in / {} out",
                    format_bytes(resp_bytes as u64),
                    format_bytes(req_bytes as u64)
                ),
            );
        }
    }

    pub fn record_ws_rx(&self, bytes: usize) {
        self.ws_rx.fetch_add(bytes as u64, Ordering::Relaxed);
        self.ws_msgs.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_ws_tx(&self, bytes: usize) {
        self.ws_tx.fetch_add(bytes as u64, Ordering::Relaxed);
        self.ws_msgs.fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            fps: load_f32(&self.fps),
            fps_smooth: load_f32(&self.fps_smooth),
            prepare_ms: load_f32(&self.prepare_ms),
            draw_ms: load_f32(&self.draw_ms),
            gl_ms: load_f32(&self.gl_ms),
            rss_bytes: self.rss_bytes.load(Ordering::Relaxed),
            vmsize_bytes: self.vmsize_bytes.load(Ordering::Relaxed),
            http_tx: self.http_tx.load(Ordering::Relaxed),
            http_rx: self.http_rx.load(Ordering::Relaxed),
            http_count: self.http_count.load(Ordering::Relaxed),
            http_tx_per_s: self.http_tx_per_s.load(Ordering::Relaxed),
            http_rx_per_s: self.http_rx_per_s.load(Ordering::Relaxed),
            http_count_per_s: self.http_count_per_s.load(Ordering::Relaxed),
            ws_tx: self.ws_tx.load(Ordering::Relaxed),
            ws_rx: self.ws_rx.load(Ordering::Relaxed),
            ws_msgs: self.ws_msgs.load(Ordering::Relaxed),
            ws_tx_per_s: self.ws_tx_per_s.load(Ordering::Relaxed),
            ws_rx_per_s: self.ws_rx_per_s.load(Ordering::Relaxed),
            ws_msgs_per_s: self.ws_msgs_per_s.load(Ordering::Relaxed),
        }
    }

    pub fn recent_ops(&self) -> Vec<OpRecord> {
        self.ops
            .lock()
            .map(|ops| ops.iter().rev().take(40).cloned().collect())
            .unwrap_or_default()
    }

    fn refresh_rates(&self) {
        let Ok(mut mark) = self.rates.lock() else {
            return;
        };
        let elapsed = mark.at.elapsed().as_secs_f64();
        if elapsed < 1.0 {
            return;
        }
        let http_tx = self.http_tx.load(Ordering::Relaxed);
        let http_rx = self.http_rx.load(Ordering::Relaxed);
        let http_count = self.http_count.load(Ordering::Relaxed);
        let ws_tx = self.ws_tx.load(Ordering::Relaxed);
        let ws_rx = self.ws_rx.load(Ordering::Relaxed);
        let ws_msgs = self.ws_msgs.load(Ordering::Relaxed);
        let scale = 1.0 / elapsed;
        self.http_tx_per_s.store(
            ((http_tx.saturating_sub(mark.http_tx)) as f64 * scale) as u64,
            Ordering::Relaxed,
        );
        self.http_rx_per_s.store(
            ((http_rx.saturating_sub(mark.http_rx)) as f64 * scale) as u64,
            Ordering::Relaxed,
        );
        self.http_count_per_s.store(
            ((http_count.saturating_sub(mark.http_count)) as f64 * scale) as u64,
            Ordering::Relaxed,
        );
        self.ws_tx_per_s.store(
            ((ws_tx.saturating_sub(mark.ws_tx)) as f64 * scale) as u64,
            Ordering::Relaxed,
        );
        self.ws_rx_per_s.store(
            ((ws_rx.saturating_sub(mark.ws_rx)) as f64 * scale) as u64,
            Ordering::Relaxed,
        );
        self.ws_msgs_per_s.store(
            ((ws_msgs.saturating_sub(mark.ws_msgs)) as f64 * scale) as u64,
            Ordering::Relaxed,
        );
        mark.at = Instant::now();
        mark.http_tx = http_tx;
        mark.http_rx = http_rx;
        mark.http_count = http_count;
        mark.ws_tx = ws_tx;
        mark.ws_rx = ws_rx;
        mark.ws_msgs = ws_msgs;
    }

    fn refresh_memory(&self) {
        let Ok(mut at) = self.mem_at.lock() else {
            return;
        };
        if at.elapsed() < Duration::from_secs(1) {
            return;
        }
        *at = Instant::now();
        let (rss, vmsize) = process_memory();
        self.rss_bytes.store(rss, Ordering::Relaxed);
        self.vmsize_bytes.store(vmsize, Ordering::Relaxed);
    }
}

pub struct Scope {
    section: Option<Section>,
    label: &'static str,
    start: Instant,
}

impl Drop for Scope {
    fn drop(&mut self) {
        let ms = self.start.elapsed().as_secs_f32() * 1000.0;
        if let Some(section) = self.section {
            global().set_section(section, ms);
        } else if ms >= OP_THRESHOLD_MS {
            global().record_op(self.label, ms, "");
        }
    }
}

pub fn global() -> &'static Profiler {
    static PROFILER: OnceLock<Profiler> = OnceLock::new();
    PROFILER.get_or_init(Profiler::new)
}

pub fn frame(delta: f64) {
    global().frame(delta);
}

pub fn scope_section(section: Section) -> Scope {
    Scope {
        section: Some(section),
        label: match section {
            Section::Prepare => "prepare",
            Section::Draw => "draw",
            Section::Gl => "gl",
        },
        start: Instant::now(),
    }
}

pub fn record_http(
    method: &str,
    path: &str,
    elapsed: Duration,
    req_bytes: usize,
    resp_bytes: usize,
) {
    global().record_http(method, path, elapsed, req_bytes, resp_bytes);
}

pub fn record_ws_rx(bytes: usize) {
    global().record_ws_rx(bytes);
}

pub fn record_ws_tx(bytes: usize) {
    global().record_ws_tx(bytes);
}

pub fn set_qos(view: QosView) {
    global().set_qos(view);
}

pub fn note_decode(ms: f32) {
    global().note_decode(ms);
}

pub fn note_remote_stream(summary: String) {
    global().note_remote_stream(summary);
}

pub fn qos() -> QosView {
    global().qos()
}

pub fn record_media(label: &str, elapsed: Duration, jpeg_bytes: usize) {
    let ms = elapsed.as_secs_f32() * 1000.0;
    if ms >= OP_THRESHOLD_MS || jpeg_bytes >= LARGE_BODY {
        global().record_op(label, ms, format_bytes(jpeg_bytes as u64));
    }
}

pub fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let n = bytes as f64;
    if n >= GB {
        format!("{:.2} GB", n / GB)
    } else if n >= MB {
        format!("{:.2} MB", n / MB)
    } else if n >= KB {
        format!("{:.1} KB", n / KB)
    } else {
        format!("{bytes} B")
    }
}

fn store_f32(slot: &AtomicU32, value: f32) {
    slot.store(value.to_bits(), Ordering::Relaxed);
}

fn load_f32(slot: &AtomicU32) -> f32 {
    f32::from_bits(slot.load(Ordering::Relaxed))
}

#[cfg(target_os = "linux")]
fn process_memory() -> (u64, u64) {
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
        return (0, 0);
    };
    let mut rss = 0u64;
    let mut vmsize = 0u64;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            rss = parse_kb(rest) * 1024;
        } else if let Some(rest) = line.strip_prefix("VmSize:") {
            vmsize = parse_kb(rest) * 1024;
        }
    }
    (rss, vmsize)
}

#[cfg(target_os = "linux")]
fn parse_kb(text: &str) -> u64 {
    text.split_whitespace()
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

#[cfg(windows)]
fn process_memory() -> (u64, u64) {
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Threading::GetCurrentProcess;
    unsafe {
        let mut counters = PROCESS_MEMORY_COUNTERS::default();
        counters.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        if GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb).is_ok() {
            return (
                counters.WorkingSetSize as u64,
                counters.PagefileUsage as u64,
            );
        }
    }
    (0, 0)
}

#[cfg(not(any(target_os = "linux", windows)))]
fn process_memory() -> (u64, u64) {
    (0, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_byte_sizes() {
        assert_eq!(format_bytes(500), "500 B");
        assert_eq!(format_bytes(2048), "2.0 KB");
        assert_eq!(format_bytes(3 * 1024 * 1024), "3.00 MB");
    }

    #[test]
    fn records_slow_ops_only() {
        let profiler = Profiler::new();
        profiler.record_op("fast", 0.2, "");
        profiler.record_op("slow", 2.5, "detail");
        let ops = profiler.recent_ops();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].label, "slow");
    }
}
