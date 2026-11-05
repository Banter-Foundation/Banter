//! Stream watch chrome: cycle focus, idle fade, and pop-out window sizing.

use banter::{
    cycle_streamer, resolve_focused_streamer, stream_chrome_alpha, stream_popout_default_size,
    streaming_user_ids, STREAM_CHROME_FADE_SECS, STREAM_CHROME_HIDE_ALPHA, STREAM_CHROME_IDLE_SECS,
    STREAM_POPOUT_DEFAULT_H, STREAM_POPOUT_DEFAULT_W, STREAM_POPOUT_WINDOW_TITLE,
};

#[test]
fn popout_window_defaults_are_sixteen_by_nine() {
    assert_eq!(stream_popout_default_size(), (STREAM_POPOUT_DEFAULT_W, STREAM_POPOUT_DEFAULT_H));
    assert_eq!(STREAM_POPOUT_DEFAULT_W * 9, STREAM_POPOUT_DEFAULT_H * 16);
    assert_eq!(STREAM_POPOUT_WINDOW_TITLE, "Banter Stream");
}

#[test]
fn streaming_user_ids_list_local_and_remote_shares() {
    let members = [(10, true), (11, false), (12, true)];
    assert_eq!(streaming_user_ids(true, Some(99), &members), vec![99, 10, 12]);
    assert_eq!(streaming_user_ids(false, Some(99), &members), vec![10, 12]);
}

#[test]
fn cycle_and_resolve_keep_a_live_focus() {
    let live = [2, 5, 8];
    assert_eq!(resolve_focused_streamer(Some(5), &live), Some(5));
    assert_eq!(resolve_focused_streamer(Some(9), &live), Some(2));
    assert_eq!(cycle_streamer(Some(5), &live), Some(8));
    assert_eq!(cycle_streamer(Some(8), &live), Some(2));
}

#[test]
fn fullscreen_chrome_stays_up_until_idle_then_fades() {
    assert_eq!(stream_chrome_alpha(0.0, true), 1.0);
    assert!(stream_chrome_alpha(STREAM_CHROME_IDLE_SECS + STREAM_CHROME_FADE_SECS, true) <= STREAM_CHROME_HIDE_ALPHA);
    assert_eq!(stream_chrome_alpha(30.0, false), 1.0);
}
