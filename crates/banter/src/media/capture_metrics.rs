//! Per-stage screen-capture counters for Linux FPS work (and shared convert/encode).
//!
//! Atomically updated from the PipeWire callback / convert / encode threads.
//! Percentile samples use a try-lock ring so a busy profiler snapshot never stalls
//! the capture callback. See `docs/linux-streaming-capture-optimization.md`.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

const SAMPLE_CAP: usize = 120;

#[derive(Clone, Debug, Default)]
pub struct CaptureSnapshot {
    pub frames_received: u64,
    pub frames_dropped: u64,
    pub frames_submitted: u64,
    pub bytes_copied: u64,
    pub callback_p50_us: u64,
    pub callback_p95_us: u64,
    pub copy_p50_us: u64,
    pub copy_p95_us: u64,
    pub convert_p50_us: u64,
    pub convert_p95_us: u64,
    pub encode_p50_us: u64,
    pub encode_p95_us: u64,
    pub capture_to_encode_p50_us: u64,
    pub capture_to_encode_p95_us: u64,
    /// Rolling captured FPS from received-frame timestamps (0 if idle).
    pub capture_fps: f32,
    /// Rolling convert-submit FPS.
    pub submit_fps: f32,
}

impl CaptureSnapshot {
    /// Keep p50 fields reachable for external benchmark tools / future UI.
    #[allow(dead_code)]
    pub fn p50_line(&self) -> String {
        format!(
            "p50 cb {:.2}ms copy {:.2}ms convert {:.2}ms encode {:.2}ms e2e {:.2}ms",
            self.callback_p50_us as f32 / 1000.0,
            self.copy_p50_us as f32 / 1000.0,
            self.convert_p50_us as f32 / 1000.0,
            self.encode_p50_us as f32 / 1000.0,
            self.capture_to_encode_p50_us as f32 / 1000.0,
        )
    }
}

struct TimingRing {
    samples_us: Mutex<VecDeque<u64>>,
}

impl TimingRing {
    fn new() -> Self {
        Self {
            samples_us: Mutex::new(VecDeque::with_capacity(SAMPLE_CAP)),
        }
    }

    fn push(&self, duration: Duration) {
        let us = duration.as_micros().min(u64::MAX as u128) as u64;
        let Ok(mut samples) = self.samples_us.try_lock() else {
            // Never block the capture callback on a stuck snapshot reader.
            return;
        };
        if samples.len() >= SAMPLE_CAP {
            samples.pop_front();
        }
        samples.push_back(us);
    }

    fn percentiles(&self) -> (u64, u64) {
        let Ok(samples) = self.samples_us.lock() else {
            return (0, 0);
        };
        if samples.is_empty() {
            return (0, 0);
        }
        let mut sorted: Vec<u64> = samples.iter().copied().collect();
        sorted.sort_unstable();
        let p50 = percentile(&sorted, 50);
        let p95 = percentile(&sorted, 95);
        (p50, p95)
    }
}

fn percentile(sorted: &[u64], pct: usize) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    // Nearest-rank: ceil(pct/100 * n) - 1, clamped.
    let n = sorted.len();
    let rank = ((pct * n + 99) / 100).saturating_sub(1).min(n - 1);
    sorted[rank]
}

struct CaptureMetrics {
    frames_received: AtomicU64,
    frames_dropped: AtomicU64,
    frames_submitted: AtomicU64,
    bytes_copied: AtomicU64,
    last_receive_ns: AtomicU64,
    receive_interval_sum_ns: AtomicU64,
    receive_interval_count: AtomicU64,
    last_submit_ns: AtomicU64,
    submit_interval_sum_ns: AtomicU64,
    submit_interval_count: AtomicU64,
    callback: TimingRing,
    copy: TimingRing,
    convert: TimingRing,
    encode: TimingRing,
    capture_to_encode: TimingRing,
}

impl CaptureMetrics {
    fn new() -> Self {
        Self {
            frames_received: AtomicU64::new(0),
            frames_dropped: AtomicU64::new(0),
            frames_submitted: AtomicU64::new(0),
            bytes_copied: AtomicU64::new(0),
            last_receive_ns: AtomicU64::new(0),
            receive_interval_sum_ns: AtomicU64::new(0),
            receive_interval_count: AtomicU64::new(0),
            last_submit_ns: AtomicU64::new(0),
            submit_interval_sum_ns: AtomicU64::new(0),
            submit_interval_count: AtomicU64::new(0),
            callback: TimingRing::new(),
            copy: TimingRing::new(),
            convert: TimingRing::new(),
            encode: TimingRing::new(),
            capture_to_encode: TimingRing::new(),
        }
    }
}

fn global() -> &'static CaptureMetrics {
    static METRICS: OnceLock<CaptureMetrics> = OnceLock::new();
    METRICS.get_or_init(CaptureMetrics::new)
}

fn now_ns() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

fn note_interval(last: &AtomicU64, sum: &AtomicU64, count: &AtomicU64) {
    let now = now_ns();
    let prev = last.swap(now, Ordering::Relaxed);
    if prev > 0 && now > prev {
        let delta = now - prev;
        // Ignore multi-second stalls (share paused) so FPS does not collapse.
        if delta < 2_000_000_000 {
            sum.fetch_add(delta, Ordering::Relaxed);
            count.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn rolling_fps(sum_ns: &AtomicU64, count: &AtomicU64) -> f32 {
    let n = count.load(Ordering::Relaxed);
    let sum = sum_ns.load(Ordering::Relaxed);
    if n == 0 || sum == 0 {
        return 0.0;
    }
    let mean_ns = (sum / n) as f64;
    if mean_ns <= 0.0 {
        return 0.0;
    }
    (1_000_000_000.0 / mean_ns) as f32
}

/// PipeWire (or other capture) delivered a buffer into the latest-wins slot.
pub fn note_received(callback: Duration, copy: Duration, bytes: u64, dropped_previous: bool) {
    let m = global();
    m.frames_received.fetch_add(1, Ordering::Relaxed);
    m.bytes_copied.fetch_add(bytes, Ordering::Relaxed);
    if dropped_previous {
        m.frames_dropped.fetch_add(1, Ordering::Relaxed);
    }
    note_interval(
        &m.last_receive_ns,
        &m.receive_interval_sum_ns,
        &m.receive_interval_count,
    );
    m.callback.push(callback);
    m.copy.push(copy);
}

/// A pending convert/encode job was overwritten (latest-wins) before use.
pub fn note_stale_drop() {
    global().frames_dropped.fetch_add(1, Ordering::Relaxed);
}

/// Convert thread published an I420 frame to the encoder slot.
pub fn note_converted(convert: Duration) {
    let m = global();
    m.frames_submitted.fetch_add(1, Ordering::Relaxed);
    note_interval(
        &m.last_submit_ns,
        &m.submit_interval_sum_ns,
        &m.submit_interval_count,
    );
    m.convert.push(convert);
}

/// Screen H.264 encode finished for one layer AU.
pub fn note_encoded(encode: Duration, capture_to_encode: Option<Duration>) {
    let m = global();
    m.encode.push(encode);
    if let Some(e2e) = capture_to_encode {
        m.capture_to_encode.push(e2e);
    }
}

/// Reset counters when Go Live stops so the next session starts clean.
pub fn clear() {
    let m = global();
    m.frames_received.store(0, Ordering::Relaxed);
    m.frames_dropped.store(0, Ordering::Relaxed);
    m.frames_submitted.store(0, Ordering::Relaxed);
    m.bytes_copied.store(0, Ordering::Relaxed);
    m.last_receive_ns.store(0, Ordering::Relaxed);
    m.receive_interval_sum_ns.store(0, Ordering::Relaxed);
    m.receive_interval_count.store(0, Ordering::Relaxed);
    m.last_submit_ns.store(0, Ordering::Relaxed);
    m.submit_interval_sum_ns.store(0, Ordering::Relaxed);
    m.submit_interval_count.store(0, Ordering::Relaxed);
    if let Ok(mut s) = m.callback.samples_us.lock() {
        s.clear();
    }
    if let Ok(mut s) = m.copy.samples_us.lock() {
        s.clear();
    }
    if let Ok(mut s) = m.convert.samples_us.lock() {
        s.clear();
    }
    if let Ok(mut s) = m.encode.samples_us.lock() {
        s.clear();
    }
    if let Ok(mut s) = m.capture_to_encode.samples_us.lock() {
        s.clear();
    }
}

pub fn snapshot() -> CaptureSnapshot {
    let m = global();
    let (callback_p50_us, callback_p95_us) = m.callback.percentiles();
    let (copy_p50_us, copy_p95_us) = m.copy.percentiles();
    let (convert_p50_us, convert_p95_us) = m.convert.percentiles();
    let (encode_p50_us, encode_p95_us) = m.encode.percentiles();
    let (capture_to_encode_p50_us, capture_to_encode_p95_us) = m.capture_to_encode.percentiles();
    CaptureSnapshot {
        frames_received: m.frames_received.load(Ordering::Relaxed),
        frames_dropped: m.frames_dropped.load(Ordering::Relaxed),
        frames_submitted: m.frames_submitted.load(Ordering::Relaxed),
        bytes_copied: m.bytes_copied.load(Ordering::Relaxed),
        callback_p50_us,
        callback_p95_us,
        copy_p50_us,
        copy_p95_us,
        convert_p50_us,
        convert_p95_us,
        encode_p50_us,
        encode_p95_us,
        capture_to_encode_p50_us,
        capture_to_encode_p95_us,
        capture_fps: rolling_fps(&m.receive_interval_sum_ns, &m.receive_interval_count),
        submit_fps: rolling_fps(&m.submit_interval_sum_ns, &m.submit_interval_count),
    }
}

/// One-line summary for `/debugnow` / F3 QoS.
pub fn summary_line() -> String {
    let s = snapshot();
    if s.frames_received == 0 && s.frames_submitted == 0 {
        return String::new();
    }
    let drop_pct = if s.frames_received == 0 {
        0.0
    } else {
        (s.frames_dropped as f64) * 100.0 / (s.frames_received as f64)
    };
    format!(
        "capture {:.0}/{:.0} fps  drop {:.1}%  cb p95 {:.2}ms  copy p95 {:.2}ms  convert p95 {:.2}ms  encode p95 {:.2}ms  e2e p95 {:.2}ms  copied {}",
        s.capture_fps,
        s.submit_fps,
        drop_pct,
        s.callback_p95_us as f32 / 1000.0,
        s.copy_p95_us as f32 / 1000.0,
        s.convert_p95_us as f32 / 1000.0,
        s.encode_p95_us as f32 / 1000.0,
        s.capture_to_encode_p95_us as f32 / 1000.0,
        crate::profiler::format_bytes(s.bytes_copied),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_basic() {
        let samples = [10u64, 20, 30, 40, 50];
        assert_eq!(percentile(&samples, 50), 30);
        assert_eq!(percentile(&samples, 95), 50);
    }

    #[test]
    fn notes_accumulate() {
        clear();
        note_received(Duration::from_micros(100), Duration::from_micros(50), 1024, false);
        note_received(Duration::from_micros(120), Duration::from_micros(60), 1024, true);
        note_converted(Duration::from_millis(2));
        note_encoded(Duration::from_millis(3), Some(Duration::from_millis(5)));
        let s = snapshot();
        assert_eq!(s.frames_received, 2);
        assert_eq!(s.frames_dropped, 1);
        assert_eq!(s.frames_submitted, 1);
        assert_eq!(s.bytes_copied, 2048);
        clear();
        assert_eq!(snapshot().frames_received, 0);
    }
}
