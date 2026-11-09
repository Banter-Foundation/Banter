//! ImGui drawing helpers for BFMD runs in chat message rows.

use std::collections::HashSet;

use dear_imgui_rs::{fonts::FontId, Ui};

use crate::bfmd::{self, Run, Style};
use crate::skin;
use crate::theme::{CONTROL_HOVER, INPUT_BG, MUTED, TEXT_NORMAL};

const LINE_GAP: f32 = 4.0;

/// Session reveal state for spoilers in a message body.
///
/// Key is `(message_id, spoiler_index)` where `spoiler_index` counts spoiler
/// runs in document order (0-based).
pub struct SpoilerInteraction<'a> {
    pub message_id: i64,
    pub revealed: &'a mut HashSet<(i64, u32)>,
}

/// Fonts available for BFMD runs. Missing italic falls back to body; bold to bold_font.
#[derive(Clone, Copy)]
pub struct Fonts {
    pub body: Option<FontId>,
    pub bold: Option<FontId>,
    pub italic: Option<FontId>,
    pub bold_italic: Option<FontId>,
}

impl Fonts {
    fn pick(self, style: &Style) -> Option<FontId> {
        match (style.bold, style.italic) {
            (true, true) => self.bold_italic.or(self.bold).or(self.italic).or(self.body),
            (true, false) => self.bold.or(self.body),
            (false, true) => self.italic.or(self.body),
            (false, false) => self.body,
        }
    }
}

/// Height of wrapped BFMD body (no "(edited)" suffix).
pub fn measure(ui: &Ui, fonts: Fonts, source: &str, wrap_w: f32) -> f32 {
    let runs = bfmd::parse(source).runs;
    if runs.is_empty() {
        return 0.0;
    }
    layout(ui, fonts, &runs, wrap_w, None, None)
}

/// Draw BFMD body at `origin`; returns height used.
///
/// When `spoilers` is provided, `{sp}` runs stay hidden until clicked; the click
/// inserts `(message_id, spoiler_index)` into `revealed` for the rest of the session.
pub fn draw(
    ui: &Ui,
    fonts: Fonts,
    source: &str,
    origin: [f32; 2],
    wrap_w: f32,
    edited_suffix: &str,
    spoilers: Option<SpoilerInteraction<'_>>,
) -> f32 {
    let mut runs = bfmd::parse(source).runs;
    if !edited_suffix.is_empty() {
        if let Some(last) = runs.last_mut() {
            last.text.push_str(edited_suffix);
        } else {
            runs.push(Run {
                text: edited_suffix.to_string(),
                style: Style::default(),
            });
        }
    }
    if runs.is_empty() {
        return 0.0;
    }
    layout(ui, fonts, &runs, wrap_w, Some(origin), spoilers)
}

fn layout(
    ui: &Ui,
    fonts: Fonts,
    runs: &[Run],
    wrap_w: f32,
    origin: Option<[f32; 2]>,
    mut spoilers: Option<SpoilerInteraction<'_>>,
) -> f32 {
    let wrap_w = wrap_w.max(8.0);
    let mut x = 0.0f32;
    let mut y = 0.0f32;
    let mut line_h = 0.0f32;
    let mut saw_hr = false;
    let mut spoiler_index = 0u32;

    for run in runs {
        if run.style.hr && !saw_hr {
            // Full-width rule for the paragraph while hr is on.
            if x > 0.0 {
                y += line_h + LINE_GAP;
                x = 0.0;
                line_h = 0.0;
            }
            if let Some(origin) = origin {
                let color = run.style.color.map(|c| c.to_imgui()).unwrap_or(
                    crate::theme::DIVIDER,
                );
                ui.get_window_draw_list()
                    .add_line(
                        [origin[0], origin[1] + y + 8.0],
                        [origin[0] + wrap_w, origin[1] + y + 8.0],
                        color,
                    )
                    .thickness(1.0)
                    .build();
            }
            y += 16.0;
            saw_hr = true;
        } else if !run.style.hr {
            saw_hr = false;
        }

        let font = fonts.pick(&run.style);
        let size = run.style.size_or_default();
        let color = run_color(&run.style);

        let spoiler_id = if run.style.spoiler {
            let id = spoiler_index;
            spoiler_index += 1;
            Some(id)
        } else {
            None
        };

        // Word-wrap within the run, carrying x across runs on the same line.
        let mut pending = String::new();
        let chars: Vec<char> = run.text.chars().collect();
        let mut i = 0usize;
        while i < chars.len() {
            if chars[i] == '\n' {
                flush_chunk(
                    ui,
                    font,
                    size,
                    color,
                    &run.style,
                    &pending,
                    origin,
                    spoiler_id,
                    spoilers.as_mut(),
                    &mut x,
                    &mut y,
                    &mut line_h,
                );
                pending.clear();
                y += line_h.max(size) + LINE_GAP;
                x = 0.0;
                line_h = 0.0;
                i += 1;
                continue;
            }
            // Take next word including leading spaces clustered with it.
            let start = i;
            if chars[i].is_whitespace() {
                while i < chars.len() && chars[i].is_whitespace() && chars[i] != '\n' {
                    i += 1;
                }
            } else {
                while i < chars.len() && !chars[i].is_whitespace() {
                    i += 1;
                }
            }
            let word: String = chars[start..i].iter().collect();
            let candidate = format!("{pending}{word}");
            let width = skin::text_size(ui, font, size, &candidate)[0];
            if x > 0.0
                && pending.is_empty()
                && width > wrap_w - x
                && !word.chars().all(|c| c.is_whitespace())
            {
                // Word alone does not fit on remainder — new line first.
                y += line_h.max(size) + LINE_GAP;
                x = 0.0;
                line_h = 0.0;
            }
            let width_from_line = skin::text_size(ui, font, size, &format!("{pending}{word}"))[0];
            if x + width_from_line > wrap_w && !pending.is_empty() {
                flush_chunk(
                    ui,
                    font,
                    size,
                    color,
                    &run.style,
                    &pending,
                    origin,
                    spoiler_id,
                    spoilers.as_mut(),
                    &mut x,
                    &mut y,
                    &mut line_h,
                );
                pending.clear();
                y += line_h.max(size) + LINE_GAP;
                x = 0.0;
                line_h = 0.0;
            }
            pending.push_str(&word);
        }
        flush_chunk(
            ui,
            font,
            size,
            color,
            &run.style,
            &pending,
            origin,
            spoiler_id,
            spoilers.as_mut(),
            &mut x,
            &mut y,
            &mut line_h,
        );
    }
    y + line_h.max(0.0)
}

fn run_color(style: &Style) -> [f32; 4] {
    if let Some(c) = style.color {
        return c.to_imgui();
    }
    if style.is_subtext() {
        MUTED
    } else {
        TEXT_NORMAL
    }
}

#[allow(clippy::too_many_arguments)]
fn flush_chunk(
    ui: &Ui,
    font: Option<FontId>,
    size: f32,
    color: [f32; 4],
    style: &Style,
    text: &str,
    origin: Option<[f32; 2]>,
    spoiler_id: Option<u32>,
    spoilers: Option<&mut SpoilerInteraction<'_>>,
    x: &mut f32,
    y: &mut f32,
    line_h: &mut f32,
) {
    if text.is_empty() {
        return;
    }
    let measured = skin::text_size(ui, font, size, text);
    *line_h = (*line_h).max(measured[1].max(size));
    if let Some(origin) = origin {
        let pos = [origin[0] + *x, origin[1] + *y];
        let hidden = style.spoiler
            && spoiler_id.is_some_and(|sid| {
                spoilers
                    .as_ref()
                    .map(|s| !s.revealed.contains(&(s.message_id, sid)))
                    .unwrap_or(true)
            });

        if let Some(bg) = style.background {
            if !hidden {
                ui.get_window_draw_list()
                    .add_rect(
                        pos,
                        [pos[0] + measured[0], pos[1] + measured[1]],
                        bg.to_imgui(),
                    )
                    .filled(true)
                    .build();
            }
        }

        if hidden {
            let hit_w = measured[0].max(8.0);
            let hit_h = measured[1].max(size);
            let max = [pos[0] + hit_w, pos[1] + hit_h];
            let hovered = pointer_in(ui, pos, max);
            let fill = if hovered { CONTROL_HOVER } else { INPUT_BG };
            ui.get_window_draw_list()
                .add_rect(pos, max, fill)
                .rounding(3.0)
                .filled(true)
                .build();
            if let (Some(sid), Some(state)) = (spoiler_id, spoilers) {
                // Invisible hit target — click reveals for this session.
                // Include pixel position so wrapped chunks of one spoiler run stay unique.
                ui.set_cursor_screen_pos(pos);
                let label = format!(
                    "bfmd-spoiler-{}-{}-{}-{}",
                    state.message_id,
                    sid,
                    pos[0] as i32,
                    pos[1] as i32
                );
                if ui.invisible_button(label, [hit_w, hit_h]) {
                    state.revealed.insert((state.message_id, sid));
                }
            }
        } else {
            skin::text(ui, font, size, pos, color, text);
            if style.underline {
                let uy = pos[1] + measured[1] - 2.0;
                ui.get_window_draw_list()
                    .add_line([pos[0], uy], [pos[0] + measured[0], uy], color)
                    .thickness(1.0)
                    .build();
            }
            if style.strike {
                let sy = pos[1] + measured[1] * 0.5;
                ui.get_window_draw_list()
                    .add_line([pos[0], sy], [pos[0] + measured[0], sy], color)
                    .thickness(1.0)
                    .build();
            }
        }
    }
    *x += measured[0];
}

fn pointer_in(ui: &Ui, min: [f32; 2], max: [f32; 2]) -> bool {
    let p = ui.io().mouse_pos();
    p[0] >= min[0] && p[0] <= max[0] && p[1] >= min[1] && p[1] <= max[1]
}
