//! Measured screen capture→publish lag, used to delay stream audio so Go Live
//! A/V stay aligned when convert/encode cannot hold 60 FPS.
//!
//! Cap is 5 seconds (Adam): stream audio may trail video by at most that long.

use std::sync::atomic::{AtomicU64, Ordering};

/// Hard ceiling on how far stream audio may lag the screen path.
pub const STREAM_AUDIO_MAX_LAG_MS: u64 = 5_000;

static SCREEN_PIPELINE_LAG_MS: AtomicU64 = AtomicU64::new(0);

/// Smooth in a new grab→publish sample (EMA). Values above the cap are clamped.
pub fn note_pipeline_lag_ms(sample_ms: u64) {
    let sample = sample_ms.min(STREAM_AUDIO_MAX_LAG_MS);
    let old = SCREEN_PIPELINE_LAG_MS.load(Ordering::Relaxed);
    let next = if sample >= STREAM_AUDIO_MAX_LAG_MS {
        // Hit the ceiling immediately so stream audio can catch a multi-second stall.
        STREAM_AUDIO_MAX_LAG_MS
    } else if old == 0 {
        sample
    } else {
        (old.saturating_mul(3).saturating_add(sample)) / 4
    };
    SCREEN_PIPELINE_LAG_MS.store(next.min(STREAM_AUDIO_MAX_LAG_MS), Ordering::Relaxed);
}

/// Clear when Go Live stops so mic-only voice is not delayed.
pub fn clear_pipeline_lag() {
    SCREEN_PIPELINE_LAG_MS.store(0, Ordering::Relaxed);
}

pub fn pipeline_lag_ms() -> u64 {
    SCREEN_PIPELINE_LAG_MS
        .load(Ordering::Relaxed)
        .min(STREAM_AUDIO_MAX_LAG_MS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lag_clamps_and_clears() {
        clear_pipeline_lag();
        note_pipeline_lag_ms(100);
        assert!(pipeline_lag_ms() >= 50);
        note_pipeline_lag_ms(10_000);
        assert_eq!(pipeline_lag_ms(), STREAM_AUDIO_MAX_LAG_MS);
        clear_pipeline_lag();
        assert_eq!(pipeline_lag_ms(), 0);
    }
}
