//! ImGui drawing helpers for BFMD runs in chat message rows.

use dear_imgui_rs::{fonts::FontId, Ui};

use crate::bfmd::{self, Run, Style};
use crate::skin;
use crate::theme::{DIVIDER, LINK, TEXT_NORMAL};

const LINE_GAP: f32 = 4.0;

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

/// Result of painting a BFMD body. A clicked masked link never opens here —
/// the caller **must** show a confirm popup with the real URL first.
#[derive(Clone, Debug, Default)]
pub struct DrawResult {
    pub height: f32,
    /// Destination URL when the user clicked a masked-link run this frame.
    pub clicked_link: Option<String>,
}

/// Height of wrapped BFMD body (no "(edited)" suffix).
pub fn measure(ui: &Ui, fonts: Fonts, source: &str, wrap_w: f32) -> f32 {
    let runs = bfmd::parse(source).runs;
    if runs.is_empty() {
        return 0.0;
    }
    layout(ui, fonts, &runs, wrap_w, None, 0).height
}

/// Draw BFMD body at `origin`; returns height and any masked-link click.
///
/// `interaction_id` namespaces invisible hit buttons (use the message id).
/// Clicking a link only reports `clicked_link` — it does **not** navigate.
pub fn draw(
    ui: &Ui,
    fonts: Fonts,
    source: &str,
    origin: [f32; 2],
    wrap_w: f32,
    edited_suffix: &str,
    interaction_id: i64,
) -> DrawResult {
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
        return DrawResult::default();
    }
    layout(ui, fonts, &runs, wrap_w, Some(origin), interaction_id)
}

fn layout(
    ui: &Ui,
    fonts: Fonts,
    runs: &[Run],
    wrap_w: f32,
    origin: Option<[f32; 2]>,
    interaction_id: i64,
) -> DrawResult {
    let wrap_w = wrap_w.max(8.0);
    let mut x = 0.0f32;
    let mut y = 0.0f32;
    let mut line_h = 0.0f32;
    let mut saw_hr = false;
    let mut clicked_link = None;
    let mut hit_seq = 0u32;

    for run in runs {
        if run.style.hr && !saw_hr {
            // Full-width rule for the paragraph while hr is on.
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
        } else if !run.style.hr {
            saw_hr = false;
        }

        let font = fonts.pick(&run.style);
        let size = run.style.size_or_default();
        let color = if run.style.link.is_some() && run.style.color.is_none() {
            LINK
        } else {
            run.style.color.map(|c| c.to_imgui()).unwrap_or(TEXT_NORMAL)
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
                    &mut x,
                    &mut y,
                    &mut line_h,
                    interaction_id,
                    &mut hit_seq,
                    &mut clicked_link,
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
                    &mut x,
                    &mut y,
                    &mut line_h,
                    interaction_id,
                    &mut hit_seq,
                    &mut clicked_link,
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
            &mut x,
            &mut y,
            &mut line_h,
            interaction_id,
            &mut hit_seq,
            &mut clicked_link,
        );
    }
    DrawResult {
        height: y + line_h.max(0.0),
        clicked_link,
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
    x: &mut f32,
    y: &mut f32,
    line_h: &mut f32,
    interaction_id: i64,
    hit_seq: &mut u32,
    clicked_link: &mut Option<String>,
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
        let underline = style.underline || style.link.is_some();
        if underline {
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
        if let Some(url) = style.link.as_ref() {
            // Hit target only — never open from paint; UI shows confirm popup first.
            ui.set_cursor_screen_pos(pos);
            let id = format!("##bfmd-link-{interaction_id}-{hit_seq}");
            *hit_seq = hit_seq.saturating_add(1);
            if ui.invisible_button(&id, [measured[0].max(1.0), measured[1].max(1.0)]) {
                if clicked_link.is_none() {
                    if let Some(pending) = bfmd::pending_masked_link_open(url) {
                        *clicked_link = Some(pending);
                    }
                }
            }
            skin::item_tooltip(ui, font, url);
        }
    }
    *x += measured[0];
}
