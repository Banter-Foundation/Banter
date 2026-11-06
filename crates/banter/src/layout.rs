//! Signed-in shell layout regions.
//!
//! Banter's window is a **fixed column shell** painted by `ui.rs`. The current look is a
//! temporary Discord-like skin. This module names the regions and computes their rectangles
//! from `theme` tokens so later theme work can change widths in one place without hunting
//! through draw code.
//!
//! Module map for UI iteration:
//! - `theme` — colors, sizes, radii, fonts (tokens only)
//! - `layout` — named regions and column geometry (this file)
//! - `skin` — shared Dear ImGui draw helpers
//! - `ui` — screens that compose regions; do not hard-code column widths here
//!
//! See `docs/ui-theme-layout.md`.

use crate::theme::{
    COMPOSER_H, HEADER_H, MEMBERS_W, RAIL_W, SIDEBAR_W, USER_PANEL_H, VOICE_PANEL_H,
};

/// Named areas of the temporary Discord-style shell.
///
/// Left-to-right columns are Rail → Sidebar → Stage → Members. Header spans the stage
/// (and members when shown). User and voice panels sit at the bottom of the sidebar.
/// The composer sits at the bottom of the stage in a text channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Region {
    Rail,
    Sidebar,
    Stage,
    Members,
    Header,
    UserPanel,
    VoicePanel,
    Composer,
}

impl Region {
    /// Stable slug for docs, tests, and future telemetry.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Rail => "rail",
            Self::Sidebar => "sidebar",
            Self::Stage => "stage",
            Self::Members => "members",
            Self::Header => "header",
            Self::UserPanel => "user-panel",
            Self::VoicePanel => "voice-panel",
            Self::Composer => "composer",
        }
    }

    /// Every region, in the order agents should learn the shell.
    pub const ALL: [Region; 8] = [
        Self::Rail,
        Self::Sidebar,
        Self::Stage,
        Self::Members,
        Self::Header,
        Self::UserPanel,
        Self::VoicePanel,
        Self::Composer,
    ];
}

/// Column widths for the Discord-style shell. Left to right: server rail, channel sidebar, stage, members.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShellColumns {
    pub rail_x: f32,
    pub rail_w: f32,
    pub sidebar_x: f32,
    pub sidebar_w: f32,
    pub stage_x: f32,
    pub stage_w: f32,
    pub members_x: f32,
    pub members_w: f32,
}

impl ShellColumns {
    /// Same as [`shell_columns`].
    pub fn from_width(width: f32, show_members: bool) -> Self {
        shell_columns(width, show_members)
    }

    /// Axis-aligned rect as `[x, y, w, h]` for a named region inside a window of `height`.
    ///
    /// Column regions fill the full height. Header sits on the stage (+ members when shown).
    /// User and voice panels stack at the bottom of the sidebar. Composer sits at the bottom
    /// of the stage. These rects match the temporary Discord skin; Banter's final layout may
    /// redefine them without changing the `Region` names.
    pub fn rect(self, region: Region, height: f32) -> [f32; 4] {
        let height = height.max(0.0);
        match region {
            Region::Rail => [self.rail_x, 0.0, self.rail_w, height],
            Region::Sidebar => [self.sidebar_x, 0.0, self.sidebar_w, height],
            Region::Stage => [self.stage_x, 0.0, self.stage_w, height],
            Region::Members => [
                self.members_x,
                HEADER_H,
                self.members_w,
                (height - HEADER_H).max(0.0),
            ],
            Region::Header => [
                self.stage_x,
                0.0,
                self.stage_w + self.members_w,
                HEADER_H.min(height),
            ],
            Region::UserPanel => {
                let h = USER_PANEL_H.min(height);
                [self.sidebar_x, height - h, self.sidebar_w, h]
            }
            Region::VoicePanel => {
                let user_h = USER_PANEL_H.min(height);
                let voice_h = VOICE_PANEL_H.min((height - user_h).max(0.0));
                [
                    self.sidebar_x,
                    height - user_h - voice_h,
                    self.sidebar_w,
                    voice_h,
                ]
            }
            Region::Composer => {
                let h = COMPOSER_H.min(height);
                [self.stage_x, height - h, self.stage_w, h]
            }
        }
    }
}

/// Column geometry from `theme` token widths. Shrinks members, then stage, when the window is narrow.
pub fn shell_columns(width: f32, show_members: bool) -> ShellColumns {
    let width = width.max(0.0);
    let rail_w = RAIL_W.min(width);
    let sidebar_w = SIDEBAR_W.min((width - rail_w).max(0.0));
    let mut members_w = if show_members {
        MEMBERS_W.min((width - rail_w - sidebar_w).max(0.0))
    } else {
        0.0
    };
    let mut stage_w = width - rail_w - sidebar_w - members_w;
    if stage_w < 0.0 {
        members_w = (members_w + stage_w).max(0.0);
        stage_w = 0.0;
    }
    let rail_x = 0.0;
    let sidebar_x = rail_x + rail_w;
    let stage_x = sidebar_x + sidebar_w;
    let members_x = stage_x + stage_w;
    ShellColumns {
        rail_x,
        rail_w,
        sidebar_x,
        sidebar_w,
        stage_x,
        stage_w,
        members_x,
        members_w,
    }
}

/// Alias kept for existing call sites and tests. Prefer [`shell_columns`] in new code.
pub fn discord_shell_columns(width: f32, show_members: bool) -> ShellColumns {
    shell_columns(width, show_members)
}
