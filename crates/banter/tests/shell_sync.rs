//! Shell polling stays off the UI thread. A snapshot applies only for the generation
//! that is still current, and a follow-up waits until the in-flight job finishes.

use std::time::{Duration, Instant};

use banter::{
    avatar_fetch_batch, first_shell_channel, shell_poll_due, snapshot_applies, ShellGate,
};
use banter_server::{Category, CategoryLayout, Channel, ChannelKind};

fn channel(id: i64, kind: ChannelKind) -> Channel {
    Channel {
        id,
        server_id: 1,
        category_id: 1,
        name: format!("c{id}"),
        kind,
        position: id,
        icon: None,
        created_by_plugin: None,
    }
}

fn layout(channels: Vec<Channel>) -> Vec<CategoryLayout> {
    vec![CategoryLayout {
        category: Category {
            id: 1,
            server_id: 1,
            name: "general".into(),
            position: 0,
        },
        channels,
    }]
}

#[test]
fn shell_poll_waits_two_seconds_and_skips_while_in_flight() {
    let start = Instant::now();
    assert!(!shell_poll_due(start, start, false));
    assert!(!shell_poll_due(
        start,
        start + Duration::from_millis(1_999),
        false
    ));
    assert!(shell_poll_due(start, start + Duration::from_secs(2), false));
    assert!(!shell_poll_due(start, start + Duration::from_secs(3), true));
}

#[test]
fn snapshot_applies_only_for_the_current_generation() {
    assert!(snapshot_applies(1, 1));
    assert!(!snapshot_applies(1, 2));
    assert!(!snapshot_applies(0, 1));
}

#[test]
fn shell_gate_runs_one_job_and_collapses_follow_ups() {
    let mut gate = ShellGate::new();
    assert!(gate.begin());
    assert!(gate.in_flight());
    assert!(!gate.begin());
    assert!(!gate.begin());
    assert!(gate.finish());
    assert!(!gate.in_flight());
    assert!(gate.begin());
    assert!(gate.in_flight());
    assert!(!gate.finish());
    assert!(!gate.in_flight());
}

#[test]
fn shell_gate_clear_pending_skips_the_follow_up() {
    let mut gate = ShellGate::new();
    assert!(gate.begin());
    assert!(!gate.begin());
    gate.clear_pending();
    assert!(!gate.finish());
    assert!(!gate.in_flight());
}

#[test]
fn avatar_fetch_batch_caps_at_four() {
    let wanted = [1, 2, 3, 4, 5, 6];
    assert_eq!(avatar_fetch_batch(&wanted), &[1, 2, 3, 4]);
    assert_eq!(avatar_fetch_batch(&wanted[..2]), &[1, 2]);
    assert!(avatar_fetch_batch::<i32>(&[]).is_empty());
}

#[test]
fn first_shell_channel_prefers_text_then_any() {
    let text_first = layout(vec![
        channel(2, ChannelKind::Voice),
        channel(3, ChannelKind::Text),
    ]);
    assert_eq!(
        first_shell_channel(&text_first).map(|channel| channel.id),
        Some(3)
    );
    let voice_only = layout(vec![channel(2, ChannelKind::Voice)]);
    assert_eq!(
        first_shell_channel(&voice_only).map(|channel| channel.id),
        Some(2)
    );
    assert!(first_shell_channel(&[]).is_none());
}
