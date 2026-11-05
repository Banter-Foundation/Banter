//! In-memory server log for the hidden debug window.
//!
//! Lines also go to stderr so a container log and the window stay in step.
//! The ring keeps the newest lines only. It resets when the process stops.
//!
//! Network counters track bytes the process has received (in) and sent (out)
//! over HTTP, WebSocket, and SFU media since boot. Rates are computed each time
//! `/v1/debug` is polled.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::model::{DebugLogLine, ServerTraffic};

const CAP: usize = 300;

struct LogState {
    started_ms: i64,
    lines: VecDeque<DebugLogLine>,
}

fn state() -> &'static Mutex<LogState> {
    static STATE: std::sync::OnceLock<Mutex<LogState>> = std::sync::OnceLock::new();
    STATE.get_or_init(|| {
        Mutex::new(LogState {
            started_ms: now_ms(),
            lines: VecDeque::new(),
        })
    })
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

pub fn record(level: &str, message: impl Into<String>) {
    let message = message.into();
    eprintln!("banter {level}: {message}");
    let line = DebugLogLine {
        at_ms: now_ms(),
        level: level.to_string(),
        message,
    };
    let Ok(mut guard) = state().lock() else {
        return;
    };
    if guard.lines.len() >= CAP {
        guard.lines.pop_front();
    }
    guard.lines.push_back(line);
}

pub fn recent() -> Vec<DebugLogLine> {
    state().lock().map(|guard| guard.lines.iter().cloned().collect()).unwrap_or_default()
}

pub fn uptime_ms() -> i64 {
    let started = state().lock().map(|guard| guard.started_ms).unwrap_or(0);
    now_ms().saturating_sub(started)
}

/// Resident set size in bytes. `0` when this platform has no reading.
pub fn rss_bytes() -> u64 {
    #[cfg(target_os = "linux")]
    {
        let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
            return 0;
        };
        for line in status.lines() {
            if let Some(rest) = line.strip_prefix("VmRSS:") {
                let kb = rest.split_whitespace().next().and_then(|value| value.parse::<u64>().ok()).unwrap_or(0);
                return kb.saturating_mul(1024);
            }
        }
        0
    }
    #[cfg(not(target_os = "linux"))]
    {
        0
    }
}

struct TrafficMark {
    at: Instant,
    http_in: u64,
    http_out: u64,
    http_reqs: u64,
    ws_in: u64,
    ws_out: u64,
    ws_msgs_in: u64,
    ws_msgs_out: u64,
    media_in: u64,
    media_out: u64,
    media_packets_in: u64,
    media_packets_out: u64,
}

struct Traffic {
    http_in: AtomicU64,
    http_out: AtomicU64,
    http_reqs: AtomicU64,
    ws_in: AtomicU64,
    ws_out: AtomicU64,
    ws_msgs_in: AtomicU64,
    ws_msgs_out: AtomicU64,
    media_in: AtomicU64,
    media_out: AtomicU64,
    media_packets_in: AtomicU64,
    media_packets_out: AtomicU64,
    mark: Mutex<TrafficMark>,
}

fn traffic() -> &'static Traffic {
    static TRAFFIC: std::sync::OnceLock<Traffic> = std::sync::OnceLock::new();
    TRAFFIC.get_or_init(|| Traffic {
        http_in: AtomicU64::new(0),
        http_out: AtomicU64::new(0),
        http_reqs: AtomicU64::new(0),
        ws_in: AtomicU64::new(0),
        ws_out: AtomicU64::new(0),
        ws_msgs_in: AtomicU64::new(0),
        ws_msgs_out: AtomicU64::new(0),
        media_in: AtomicU64::new(0),
        media_out: AtomicU64::new(0),
        media_packets_in: AtomicU64::new(0),
        media_packets_out: AtomicU64::new(0),
        mark: Mutex::new(TrafficMark {
            at: Instant::now(),
            http_in: 0,
            http_out: 0,
            http_reqs: 0,
            ws_in: 0,
            ws_out: 0,
            ws_msgs_in: 0,
            ws_msgs_out: 0,
            media_in: 0,
            media_out: 0,
            media_packets_in: 0,
            media_packets_out: 0,
        }),
    })
}

pub fn note_http_request() {
    traffic().http_reqs.fetch_add(1, Ordering::Relaxed);
}

pub fn add_http_in(bytes: u64) {
    if bytes > 0 {
        traffic().http_in.fetch_add(bytes, Ordering::Relaxed);
    }
}

pub fn add_http_out(bytes: u64) {
    if bytes > 0 {
        traffic().http_out.fetch_add(bytes, Ordering::Relaxed);
    }
}

pub fn record_ws_in(bytes: u64) {
    let t = traffic();
    t.ws_in.fetch_add(bytes, Ordering::Relaxed);
    t.ws_msgs_in.fetch_add(1, Ordering::Relaxed);
}

pub fn record_ws_out(bytes: u64) {
    let t = traffic();
    t.ws_out.fetch_add(bytes, Ordering::Relaxed);
    t.ws_msgs_out.fetch_add(1, Ordering::Relaxed);
}

pub fn record_media_in(bytes: u64) {
    let t = traffic();
    t.media_in.fetch_add(bytes, Ordering::Relaxed);
    t.media_packets_in.fetch_add(1, Ordering::Relaxed);
}

pub fn record_media_out(bytes: u64) {
    let t = traffic();
    t.media_out.fetch_add(bytes, Ordering::Relaxed);
    t.media_packets_out.fetch_add(1, Ordering::Relaxed);
}

fn rate(delta: u64, elapsed_secs: f64) -> u64 {
    if elapsed_secs <= 0.0 {
        return 0;
    }
    ((delta as f64) / elapsed_secs).round() as u64
}

/// Totals since boot plus rates since the previous snapshot.
pub fn traffic_snapshot() -> ServerTraffic {
    let t = traffic();
    let http_in = t.http_in.load(Ordering::Relaxed);
    let http_out = t.http_out.load(Ordering::Relaxed);
    let http_reqs = t.http_reqs.load(Ordering::Relaxed);
    let ws_in = t.ws_in.load(Ordering::Relaxed);
    let ws_out = t.ws_out.load(Ordering::Relaxed);
    let ws_msgs_in = t.ws_msgs_in.load(Ordering::Relaxed);
    let ws_msgs_out = t.ws_msgs_out.load(Ordering::Relaxed);
    let media_in = t.media_in.load(Ordering::Relaxed);
    let media_out = t.media_out.load(Ordering::Relaxed);
    let media_packets_in = t.media_packets_in.load(Ordering::Relaxed);
    let media_packets_out = t.media_packets_out.load(Ordering::Relaxed);

    let mut mark = t.mark.lock().expect("traffic mark");
    let elapsed = mark.at.elapsed().as_secs_f64().max(0.001);
    let snap = ServerTraffic {
        http_in,
        http_out,
        http_reqs,
        http_in_per_s: rate(http_in.saturating_sub(mark.http_in), elapsed),
        http_out_per_s: rate(http_out.saturating_sub(mark.http_out), elapsed),
        http_reqs_per_s: rate(http_reqs.saturating_sub(mark.http_reqs), elapsed),
        ws_in,
        ws_out,
        ws_msgs_in,
        ws_msgs_out,
        ws_in_per_s: rate(ws_in.saturating_sub(mark.ws_in), elapsed),
        ws_out_per_s: rate(ws_out.saturating_sub(mark.ws_out), elapsed),
        media_in,
        media_out,
        media_packets_in,
        media_packets_out,
        media_in_per_s: rate(media_in.saturating_sub(mark.media_in), elapsed),
        media_out_per_s: rate(media_out.saturating_sub(mark.media_out), elapsed),
    };
    mark.at = Instant::now();
    mark.http_in = http_in;
    mark.http_out = http_out;
    mark.http_reqs = http_reqs;
    mark.ws_in = ws_in;
    mark.ws_out = ws_out;
    mark.ws_msgs_in = ws_msgs_in;
    mark.ws_msgs_out = ws_msgs_out;
    mark.media_in = media_in;
    mark.media_out = media_out;
    mark.media_packets_in = media_packets_in;
    mark.media_packets_out = media_packets_out;
    snap
}

#[cfg(test)]
pub fn clear_for_test() {
    if let Ok(mut guard) = state().lock() {
        guard.lines.clear();
    }
}

#[cfg(test)]
fn clear_traffic_for_test() {
    let t = traffic();
    t.http_in.store(0, Ordering::Relaxed);
    t.http_out.store(0, Ordering::Relaxed);
    t.http_reqs.store(0, Ordering::Relaxed);
    t.ws_in.store(0, Ordering::Relaxed);
    t.ws_out.store(0, Ordering::Relaxed);
    t.ws_msgs_in.store(0, Ordering::Relaxed);
    t.ws_msgs_out.store(0, Ordering::Relaxed);
    t.media_in.store(0, Ordering::Relaxed);
    t.media_out.store(0, Ordering::Relaxed);
    t.media_packets_in.store(0, Ordering::Relaxed);
    t.media_packets_out.store(0, Ordering::Relaxed);
    if let Ok(mut mark) = t.mark.lock() {
        *mark = TrafficMark {
            at: Instant::now(),
            http_in: 0,
            http_out: 0,
            http_reqs: 0,
            ws_in: 0,
            ws_out: 0,
            ws_msgs_in: 0,
            ws_msgs_out: 0,
            media_in: 0,
            media_out: 0,
            media_packets_in: 0,
            media_packets_out: 0,
        };
    }
}

#[cfg(test)]
fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(())).lock().expect("debuglog test lock")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_keeps_newest_lines_in_order() {
        let _guard = test_lock();
        clear_for_test();
        record("info", "alpha");
        record("warn", "beta");
        let lines = recent();
        let alpha = lines.iter().find(|line| line.message == "alpha").expect("alpha");
        let beta = lines.iter().find(|line| line.message == "beta").expect("beta");
        assert_eq!(alpha.level, "info");
        assert_eq!(beta.level, "warn");
        assert!(beta.at_ms >= alpha.at_ms);
        clear_for_test();
    }

    #[test]
    fn traffic_totals_and_rates_track_in_and_out() {
        let _guard = test_lock();
        clear_traffic_for_test();
        note_http_request();
        add_http_in(100);
        add_http_out(200);
        record_ws_in(40);
        record_ws_out(60);
        record_media_in(1_000);
        record_media_out(2_000);
        record_media_out(500);
        let first = traffic_snapshot();
        assert_eq!(first.http_in, 100);
        assert_eq!(first.http_out, 200);
        assert_eq!(first.http_reqs, 1);
        assert_eq!(first.ws_in, 40);
        assert_eq!(first.ws_out, 60);
        assert_eq!(first.ws_msgs_in, 1);
        assert_eq!(first.ws_msgs_out, 1);
        assert_eq!(first.media_in, 1_000);
        assert_eq!(first.media_out, 2_500);
        assert_eq!(first.media_packets_in, 1);
        assert_eq!(first.media_packets_out, 2);
        std::thread::sleep(std::time::Duration::from_millis(20));
        note_http_request();
        add_http_in(50);
        let second = traffic_snapshot();
        assert_eq!(second.http_in, 150);
        assert_eq!(second.http_out, 200);
        assert_eq!(second.http_reqs, 2);
        assert!(second.http_in_per_s >= 1);
        assert_eq!(second.ws_in_per_s, 0);
        clear_traffic_for_test();
    }
}
