//! Layout regions for the temporary Discord shell.
//!
//! Column geometry lives in `banter::layout`. Token widths stay in `banter::theme`.

use banter::layout::{Region, ShellColumns, discord_shell_columns, shell_columns};
use banter::theme;

#[test]
fn shell_regions_are_named_left_to_right() {
    assert_eq!(Region::TitleBar.as_str(), "title-bar");
    assert_eq!(Region::Rail.as_str(), "rail");
    assert_eq!(Region::Sidebar.as_str(), "sidebar");
    assert_eq!(Region::Stage.as_str(), "stage");
    assert_eq!(Region::Members.as_str(), "members");
    assert_eq!(Region::Header.as_str(), "header");
    assert_eq!(Region::UserPanel.as_str(), "user-panel");
    assert_eq!(Region::VoicePanel.as_str(), "voice-panel");
    assert_eq!(Region::Composer.as_str(), "composer");
    assert_eq!(
        Region::ALL,
        [
            Region::TitleBar,
            Region::Rail,
            Region::Sidebar,
            Region::Stage,
            Region::Members,
            Region::Header,
            Region::UserPanel,
            Region::VoicePanel,
            Region::Composer,
        ]
    );
}

#[test]
fn shell_columns_read_widths_from_theme_tokens() {
    let columns = shell_columns(1280.0, true);
    assert_eq!(columns.rail_w, theme::RAIL_W);
    assert_eq!(columns.sidebar_w, theme::SIDEBAR_W);
    assert_eq!(columns.members_w, theme::MEMBERS_W);
    assert_eq!(columns.rail_x, 0.0);
    assert_eq!(columns.sidebar_x, theme::RAIL_W);
    assert_eq!(columns.stage_x, theme::RAIL_W + theme::SIDEBAR_W);
    assert_eq!(
        columns.stage_w,
        1280.0 - theme::RAIL_W - theme::SIDEBAR_W - theme::MEMBERS_W
    );
    assert_eq!(columns.members_x, columns.stage_x + columns.stage_w);
}

#[test]
fn discord_shell_columns_alias_matches_shell_columns() {
    let a = shell_columns(960.0, false);
    let b = discord_shell_columns(960.0, false);
    assert_eq!(a, b);
    assert_eq!(a.members_w, 0.0);
    assert_eq!(a.stage_w, 960.0 - theme::RAIL_W - theme::SIDEBAR_W);
}

#[test]
fn shell_columns_shrink_without_negative_widths() {
    let columns = shell_columns(100.0, true);
    assert!(columns.rail_w >= 0.0);
    assert!(columns.sidebar_w >= 0.0);
    assert!(columns.stage_w >= 0.0);
    assert!(columns.members_w >= 0.0);
    assert!(close(
        columns.rail_w + columns.sidebar_w + columns.stage_w + columns.members_w,
        100.0
    ));
}

#[test]
fn shell_columns_expose_each_named_region_rect() {
    let columns = ShellColumns::from_width(1280.0, true);
    let title = columns.rect(Region::TitleBar, 720.0);
    assert_eq!(title, [0.0, 0.0, 1280.0, theme::TITLE_BAR_H]);
    let rail = columns.rect(Region::Rail, 720.0);
    assert_eq!(
        rail,
        [
            0.0,
            theme::TITLE_BAR_H,
            theme::RAIL_W,
            720.0 - theme::TITLE_BAR_H
        ]
    );
    let stage = columns.rect(Region::Stage, 720.0);
    assert_eq!(stage[0], columns.stage_x);
    assert_eq!(stage[1], theme::TITLE_BAR_H);
    assert_eq!(stage[2], columns.stage_w);
    let header = columns.rect(Region::Header, 720.0);
    assert_eq!(header[1], theme::TITLE_BAR_H);
    assert_eq!(header[3], theme::HEADER_H);
    let user = columns.rect(Region::UserPanel, 720.0);
    assert!(close(user[1] + user[3], 720.0));
    assert_eq!(user[3], theme::USER_PANEL_H);
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}
