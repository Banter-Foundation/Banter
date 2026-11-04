//! In-memory server log for the hidden debug window.
//!
//! Lines also go to stderr so a container log and the window stay in step.
//! The ring keeps the newest lines only. It resets when the process stops.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::model::DebugLogLine;

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

#[cfg(test)]
pub fn clear_for_test() {
    if let Ok(mut guard) = state().lock() {
        guard.lines.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_keeps_newest_lines_in_order() {
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
}
