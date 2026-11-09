//! ImGui drawing helpers for BFMD runs in chat message rows.

use dear_imgui_rs::{fonts::FontId, Ui};

use crate::bfmd::{self, Run, Style};
use crate::skin;
use crate::theme::{DIVIDER, MUTED, TEXT_NORMAL};

const LINE_GAP: f32 = 4.0;
/// Left bar + indent for Cited Quote runs (Banter/ImGui theme).
const CITED_QUOTE_INDENT: f32 = 12.0;
const CITED_QUOTE_BAR_W: f32 = 3.0;
const CITED_QUOTE_BAR_GAP: f32 = 8.0;

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
        // Cited Quote forces italic composition even when `{i}` is off.
        let italic = style.effective_italic();
        match (style.bold, italic) {
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
    layout(ui, fonts, &runs, wrap_w, None, None).0
}

/// Draw BFMD body at `origin`; returns height used.
///
/// When a Cited Quote with a message id is clicked, `quote_jump` is set to that id
/// so the chat list can scroll to the cited message (Discord-style reply jump).
pub fn draw(
    ui: &Ui,
    fonts: Fonts,
    source: &str,
    origin: [f32; 2],
    wrap_w: f32,
    edited_suffix: &str,
    quote_jump: &mut Option<i64>,
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
    layout(ui, fonts, &runs, wrap_w, Some(origin), Some(quote_jump)).0
}

fn layout(
    ui: &Ui,
    fonts: Fonts,
    runs: &[Run],
    wrap_w: f32,
    origin: Option<[f32; 2]>,
    mut quote_jump: Option<&mut Option<i64>>,
) -> (f32, Vec<(i64, [f32; 2], [f32; 2])>) {
    let wrap_w = wrap_w.max(8.0);
    let mut x = 0.0f32;
    let mut y = 0.0f32;
    let mut line_h = 0.0f32;
    let mut saw_hr = false;
    let mut hits: Vec<(i64, [f32; 2], [f32; 2])> = Vec::new();
    let mut quote_block: Option<(i64, f32, f32, f32)> = None; // id, x0, y0, x1

    let finish_quote_block = |block: &mut Option<(i64, f32, f32, f32)>,
                             y: f32,
                             line_h: f32,
                             hits: &mut Vec<(i64, [f32; 2], [f32; 2])>| {
        if let Some((id, x0, y0, x1)) = block.take() {
            hits.push((id, [x0, y0], [x1, y + line_h.max(0.0)]));
        }
    };

    for run in runs {
        let quoting = run.style.cited_quote.is_some();
        let cite_id = run.style.cite_message_id();
        let indent = if quoting {
            CITED_QUOTE_INDENT
        } else {
            0.0
        };
        let content_wrap = (wrap_w - indent).max(8.0);

        // Starting a new cited-quote block (or switching targets): close previous hit box.
        if let Some(id) = cite_id {
            match quote_block {
                Some((cur, _, _, _)) if cur == id => {}
                _ => {
                    finish_quote_block(&mut quote_block, y, line_h, &mut hits);
                    if let Some(origin) = origin {
                        quote_block = Some((
                            id,
                            origin[0],
                            origin[1] + y,
                            origin[0] + wrap_w,
                        ));
                    }
                }
            }
        } else if quote_block.is_some() {
            finish_quote_block(&mut quote_block, y, line_h, &mut hits);
        }

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
        // Cited Quote defaults to muted foreground unless `{c}` overrides.
        let color = run
            .style
            .color
            .map(|c| c.to_imgui())
            .unwrap_or(if quoting { MUTED } else { TEXT_NORMAL });

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
                    indent,
                    &mut x,
                    &mut y,
                    &mut line_h,
                );
                pending.clear();
                // Left bar for the line we just finished.
                if quoting {
                    if let Some(origin) = origin {
                        draw_cited_quote_bar(ui, origin, y, line_h.max(size));
                    }
                }
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
            if x > indent
                && pending.is_empty()
                && width > content_wrap - (x - indent).max(0.0)
                && !word.chars().all(|c| c.is_whitespace())
            {
                // Word alone does not fit on remainder — new line first.
                if quoting {
                    if let Some(origin) = origin {
                        draw_cited_quote_bar(ui, origin, y, line_h.max(size));
                    }
                }
                y += line_h.max(size) + LINE_GAP;
                x = 0.0;
                line_h = 0.0;
            }
            let width_from_line = skin::text_size(ui, font, size, &format!("{pending}{word}"))[0];
            let line_used = if x <= 0.0 { 0.0 } else { x - indent };
            if indent + line_used + width_from_line > wrap_w && !pending.is_empty() {
                flush_chunk(
                    ui,
                    font,
                    size,
                    color,
                    &run.style,
                    &pending,
                    origin,
                    indent,
                    &mut x,
                    &mut y,
                    &mut line_h,
                );
                pending.clear();
                if quoting {
                    if let Some(origin) = origin {
                        draw_cited_quote_bar(ui, origin, y, line_h.max(size));
                    }
                }
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
            indent,
            &mut x,
            &mut y,
            &mut line_h,
        );
        if quoting && line_h > 0.0 {
            if let Some(origin) = origin {
                draw_cited_quote_bar(ui, origin, y, line_h.max(size));
            }
        }
    }
    finish_quote_block(&mut quote_block, y, line_h, &mut hits);

    // Clickable hit targets for Cited Quote → message jump.
    if let Some(origin) = origin {
        for (idx, (id, min, max)) in hits.iter().enumerate() {
            let w = (max[0] - min[0]).max(8.0);
            let h = (max[1] - min[1]).max(8.0);
            ui.set_cursor_screen_pos(*min);
            if ui.invisible_button(format!("cited-quote-{id}-{idx}-{}", origin[0] as i32), [w, h])
            {
                if let Some(jump) = quote_jump.as_mut() {
                    **jump = Some(*id);
                }
            }
        }
    }

    (y + line_h.max(0.0), hits)
}

fn draw_cited_quote_bar(ui: &Ui, origin: [f32; 2], y: f32, line_h: f32) {
    let x = origin[0] + 1.0;
    let y0 = origin[1] + y;
    let y1 = y0 + line_h.max(1.0);
    ui.get_window_draw_list()
        .add_rect(
            [x, y0],
            [x + CITED_QUOTE_BAR_W, y1],
            DIVIDER,
        )
        .filled(true)
        .rounding(1.0)
        .build();
    let _ = CITED_QUOTE_BAR_GAP;
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
    indent: f32,
    x: &mut f32,
    y: &mut f32,
    line_h: &mut f32,
) {
    if text.is_empty() {
        return;
    }
    if *x <= 0.0 && indent > 0.0 {
        *x = indent;
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
