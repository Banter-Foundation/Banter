//! Stream watching chrome: pop-out window, cycle streamers, fullscreen fade.
//!
//! Kept free of drawing so tests can check focus, cycling, and idle fade without a window.
//! The Banter Stream OS window mirrors the Banter Settings shared-GLFW pattern.

/// OS window title for a popped-out stream.
pub const STREAM_POPOUT_WINDOW_TITLE: &str = "Banter Stream";
/// Default client size for a new stream pop-out (16:9).
pub const STREAM_POPOUT_DEFAULT_W: i32 = 1280;
pub const STREAM_POPOUT_DEFAULT_H: i32 = 720;

/// Seconds without pointer motion before fullscreen chrome begins to fade.
pub const STREAM_CHROME_IDLE_SECS: f32 = 2.0;
/// Fade length after the idle delay (fullscreen only).
pub const STREAM_CHROME_FADE_SECS: f32 = 0.35;
/// Below this alpha, fullscreen chrome is not drawn or clicked.
pub const STREAM_CHROME_HIDE_ALPHA: f32 = 0.05;

/// Default client size for a new Banter Stream window.
pub fn stream_popout_default_size() -> (i32, i32) {
    (STREAM_POPOUT_DEFAULT_W, STREAM_POPOUT_DEFAULT_H)
}

/// User ids currently publishing a screen share, local first when the viewer is live.
///
/// `members` is `(user_id, screen_flag)` for everyone in the active voice/call roster.
pub fn streaming_user_ids(local_streaming: bool, me: Option<i64>, members: &[(i64, bool)]) -> Vec<i64> {
    let mut ids = Vec::new();
    if local_streaming {
        if let Some(me) = me {
            ids.push(me);
        }
    }
    for &(user_id, screen) in members {
        if !screen {
            continue;
        }
        if Some(user_id) == me && local_streaming {
            continue;
        }
        if !ids.contains(&user_id) {
            ids.push(user_id);
        }
    }
    ids
}

/// Pick which streamer to show. An absent or stale focus falls back to the first live id.
pub fn resolve_focused_streamer(focus: Option<i64>, streamers: &[i64]) -> Option<i64> {
    if streamers.is_empty() {
        return None;
    }
    if let Some(focus) = focus {
        if streamers.contains(&focus) {
            return Some(focus);
        }
    }
    streamers.first().copied()
}

/// Advance to the next live streamer (wraps). Returns the new focus, or `None` when nobody is live.
pub fn cycle_streamer(focus: Option<i64>, streamers: &[i64]) -> Option<i64> {
    if streamers.is_empty() {
        return None;
    }
    if streamers.len() == 1 {
        return streamers.first().copied();
    }
    let current = resolve_focused_streamer(focus, streamers)?;
    let index = streamers.iter().position(|id| *id == current).unwrap_or(0);
    let next = (index + 1) % streamers.len();
    Some(streamers[next])
}

/// Theater / pop-out chrome opacity from idle time.
///
/// When `fade` is false (normal featured stage) the chrome stays fully visible.
/// Fullscreen and the Banter Stream pop-out pass `true` so profile icons, voice
/// indicators, and watch controls hide together after pointer idle.
pub fn stream_chrome_alpha(idle_secs: f32, fade: bool) -> f32 {
    if !fade {
        return 1.0;
    }
    if idle_secs <= STREAM_CHROME_IDLE_SECS {
        return 1.0;
    }
    let t = ((idle_secs - STREAM_CHROME_IDLE_SECS) / STREAM_CHROME_FADE_SECS).clamp(0.0, 1.0);
    1.0 - t
}

/// Horizontal pitch between stream-watch profile icons (avatar + gap).
pub const STREAM_WATCH_AVATAR_PITCH: f32 = 40.0;

/// Left edge for the next stream-watch avatar in a bottom-left row.
pub fn stream_watch_avatar_x(origin_x: f32, index: usize) -> f32 {
    origin_x + index as f32 * STREAM_WATCH_AVATAR_PITCH
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streaming_list_puts_local_first_and_skips_duplicates() {
        let members = [(1, true), (2, true), (3, false), (1, true)];
        assert_eq!(streaming_user_ids(true, Some(1), &members), vec![1, 2]);
        assert_eq!(streaming_user_ids(false, Some(1), &members), vec![1, 2]);
        assert!(streaming_user_ids(false, Some(9), &[(3, false)]).is_empty());
    }

    #[test]
    fn focus_falls_back_when_streamer_stops() {
        let live = [4, 7, 9];
        assert_eq!(resolve_focused_streamer(Some(7), &live), Some(7));
        assert_eq!(resolve_focused_streamer(Some(1), &live), Some(4));
        assert_eq!(resolve_focused_streamer(None, &live), Some(4));
        assert_eq!(resolve_focused_streamer(Some(7), &[]), None);
    }

    #[test]
    fn cycle_wraps_across_live_streamers() {
        let live = [4, 7, 9];
        assert_eq!(cycle_streamer(Some(4), &live), Some(7));
        assert_eq!(cycle_streamer(Some(9), &live), Some(4));
        assert_eq!(cycle_streamer(None, &live), Some(7));
        assert_eq!(cycle_streamer(Some(1), &[4]), Some(4));
        assert_eq!(cycle_streamer(None, &[]), None);
    }

    #[test]
    fn fullscreen_chrome_fades_after_idle() {
        assert_eq!(stream_chrome_alpha(0.0, false), 1.0);
        assert_eq!(stream_chrome_alpha(10.0, false), 1.0);
        assert_eq!(stream_chrome_alpha(0.5, true), 1.0);
        assert_eq!(stream_chrome_alpha(STREAM_CHROME_IDLE_SECS, true), 1.0);
        assert!((stream_chrome_alpha(STREAM_CHROME_IDLE_SECS + STREAM_CHROME_FADE_SECS * 0.5, true) - 0.5).abs() < 0.01);
        assert_eq!(
            stream_chrome_alpha(STREAM_CHROME_IDLE_SECS + STREAM_CHROME_FADE_SECS + 1.0, true),
            0.0
        );
    }

    #[test]
    fn stream_watch_avatars_lay_out_left_to_right() {
        assert_eq!(stream_watch_avatar_x(10.0, 0), 10.0);
        assert_eq!(stream_watch_avatar_x(10.0, 1), 10.0 + STREAM_WATCH_AVATAR_PITCH);
        assert_eq!(stream_watch_avatar_x(10.0, 2), 10.0 + 2.0 * STREAM_WATCH_AVATAR_PITCH);
    }
}
