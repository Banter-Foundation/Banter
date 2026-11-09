//! ImGui drawing helpers for BFMD runs in chat message rows.

use dear_imgui_rs::{fonts::FontId, Ui};

use crate::bfmd::{self, ListKind, Run, Style};
use crate::skin;
use crate::theme::{DIVIDER, TEXT_NORMAL};

const LINE_GAP: f32 = 4.0;
/// Horizontal step per list nesting level (`{ld}`).
const LIST_INDENT: f32 = 20.0;

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
    layout(ui, fonts, &runs, wrap_w, None)
}

/// Draw BFMD body at `origin`; returns height used.
pub fn draw(
    ui: &Ui,
    fonts: Fonts,
    source: &str,
    origin: [f32; 2],
    wrap_w: f32,
    edited_suffix: &str,
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
    layout(ui, fonts, &runs, wrap_w, Some(origin))
}

fn layout(ui: &Ui, fonts: Fonts, runs: &[Run], wrap_w: f32, origin: Option<[f32; 2]>) -> f32 {
    let wrap_w = wrap_w.max(8.0);
    let mut x = 0.0f32;
    let mut y = 0.0f32;
    let mut line_h = 0.0f32;
    let mut saw_hr = false;
    let mut at_line_start = true;
    // Ordered item counters keyed by depth 1..=8.
    let mut ordered_counts = [0u32; 9];

    for run in runs {
        if run.style.hr && !saw_hr {
            if x > 0.0 {
                y += line_h + LINE_GAP;
                x = 0.0;
                line_h = 0.0;
            }
            if let Some(origin) = origin {
                let color = run.style.color.map(|c| c.to_imgui()).unwrap_or(DIVIDER);
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
            at_line_start = true;
        } else if !run.style.hr {
            saw_hr = false;
        }

        let font = fonts.pick(&run.style);
        let size = run.style.size_or_default();
        let color = run.style.color.map(|c| c.to_imgui()).unwrap_or(TEXT_NORMAL);

        let mut pending = String::new();
        let chars: Vec<char> = run.text.chars().collect();
        let mut i = 0usize;
        while i < chars.len() {
            if at_line_start {
                if chars[i] == '\n' {
                    // Blank line: no list marker, do not advance ordered counts.
                    if run.style.list_kind.is_none() {
                        ordered_counts = [0u32; 9];
                    }
                } else {
                    x = begin_line(
                        ui,
                        font,
                        size,
                        color,
                        &run.style,
                        origin,
                        y,
                        &mut ordered_counts,
                    );
                }
                at_line_start = false;
            }
            if chars[i] == '\n' {
                flush_chunk(
                    ui,
                    font,
                    size,
                    color,
                    &run.style,
                    &pending,
                    origin,
                    &mut x,
                    &mut y,
                    &mut line_h,
                );
                pending.clear();
                y += line_h.max(size) + LINE_GAP;
                x = 0.0;
                line_h = 0.0;
                at_line_start = true;
                i += 1;
                continue;
            }
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
            let hang = hanging_indent(&run.style);
            let candidate = format!("{pending}{word}");
            let width = skin::text_size(ui, font, size, &candidate)[0];
            if x > hang
                && pending.is_empty()
                && width > (wrap_w - x).max(8.0)
                && !word.chars().all(|c| c.is_whitespace())
            {
                y += line_h.max(size) + LINE_GAP;
                x = hang;
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
                    &mut x,
                    &mut y,
                    &mut line_h,
                );
                pending.clear();
                y += line_h.max(size) + LINE_GAP;
                x = hang;
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
            &mut x,
            &mut y,
            &mut line_h,
        );
    }
    y + line_h.max(0.0)
}

fn hanging_indent(style: &Style) -> f32 {
    let depth = style.list_depth_or_default();
    if depth == 0 {
        0.0
    } else {
        (depth as f32) * LIST_INDENT
    }
}

/// Start a visual line: draw list marker if needed; return text x offset.
fn begin_line(
    ui: &Ui,
    font: Option<FontId>,
    size: f32,
    color: [f32; 4],
    style: &Style,
    origin: Option<[f32; 2]>,
    y: f32,
    ordered_counts: &mut [u32; 9],
) -> f32 {
    let Some(kind) = style.list_kind else {
        *ordered_counts = [0u32; 9];
        return 0.0;
    };
    let depth = style.list_depth_or_default().max(1);
    let nest = (depth as f32 - 1.0) * LIST_INDENT;
    let marker = match kind {
        ListKind::Unordered => "• ".to_string(),
        ListKind::Ordered => {
            let d = depth as usize;
            for c in ordered_counts.iter_mut().skip(d + 1) {
                *c = 0;
            }
            ordered_counts[d] = ordered_counts[d].saturating_add(1);
            format!("{}. ", ordered_counts[d])
        }
    };
    let marker_w = skin::text_size(ui, font, size, &marker)[0];
    if let Some(origin) = origin {
        skin::text(
            ui,
            font,
            size,
            [origin[0] + nest, origin[1] + y],
            color,
            &marker,
        );
    }
    nest + marker_w
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
        if let Some(bg) = style.background {
            ui.get_window_draw_list()
                .add_rect(
                    pos,
                    [pos[0] + measured[0], pos[1] + measured[1]],
                    bg.to_imgui(),
                )
                .filled(true)
                .build();
        }
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
    *x += measured[0];
}
