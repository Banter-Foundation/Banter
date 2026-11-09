//! ImGui drawing helpers for BFMD runs in chat message rows.

use dear_imgui_rs::{fonts::FontId, Ui};

use crate::bfmd::{self, Block, Run, Style};
use crate::skin;
use crate::theme::{
    CODE_BLOCK_BG, CODE_BLOCK_BORDER, CODE_BLOCK_HEADER, DIVIDER, MUTED, TEXT_NORMAL, TEXT_SECONDARY,
};

const LINE_GAP: f32 = 4.0;
const BLOCK_PAD: f32 = 8.0;
const BLOCK_HEADER_H: f32 = 22.0;
const BLOCK_RADIUS: f32 = 6.0;
const BLOCK_FONT_PT: f32 = 14.0;

/// Fonts available for BFMD runs. Missing italic falls back to body; bold to bold_font.
#[derive(Clone, Copy)]
pub struct Fonts {
    pub body: Option<FontId>,
    pub bold: Option<FontId>,
    pub italic: Option<FontId>,
    pub bold_italic: Option<FontId>,
    /// Optional monospace face for `{cb}` / `{tb}` bodies.
    pub mono: Option<FontId>,
}

impl Fonts {
    fn pick(self, style: &Style) -> Option<FontId> {
        if style.block.is_some() {
            return self.mono.or(self.body);
        }
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
    layout(ui, fonts, &runs, wrap_w, None).0
}

/// Draw BFMD body at `origin`; returns `(height, clipboard text if Copy clicked)`.
pub fn draw(
    ui: &Ui,
    fonts: Fonts,
    source: &str,
    origin: [f32; 2],
    wrap_w: f32,
    edited_suffix: &str,
) -> (f32, Option<String>) {
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
        return (0.0, None);
    }
    layout(ui, fonts, &runs, wrap_w, Some(origin))
}

fn layout(
    ui: &Ui,
    fonts: Fonts,
    runs: &[Run],
    wrap_w: f32,
    origin: Option<[f32; 2]>,
) -> (f32, Option<String>) {
    let wrap_w = wrap_w.max(8.0);
    let mut x = 0.0f32;
    let mut y = 0.0f32;
    let mut line_h = 0.0f32;
    let mut saw_hr = false;
    let mut copy: Option<String> = None;
    let mut block_id = 0u32;

    for run in runs {
        if let Some(block) = &run.style.block {
            if x > 0.0 || line_h > 0.0 {
                y += line_h + LINE_GAP;
                x = 0.0;
                line_h = 0.0;
            }
            let (h, clicked) = layout_block(
                ui,
                fonts,
                block,
                &run.text,
                wrap_w,
                origin.map(|o| [o[0], o[1] + y]),
                block_id,
            );
            block_id = block_id.wrapping_add(1);
            if clicked.is_some() {
                copy = clicked;
            }
            y += h + LINE_GAP;
            saw_hr = false;
            continue;
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
        let color = run.style.color.map(|c| c.to_imgui()).unwrap_or(TEXT_NORMAL);

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
        );
    }
    (y + line_h.max(0.0), copy)
}

fn layout_block(
    ui: &Ui,
    fonts: Fonts,
    block: &Block,
    text: &str,
    wrap_w: f32,
    origin: Option<[f32; 2]>,
    id: u32,
) -> (f32, Option<String>) {
    let font = fonts.mono.or(fonts.body);
    let size = BLOCK_FONT_PT;
    let inner_w = (wrap_w - BLOCK_PAD * 2.0).max(8.0);
    let label = match block {
        Block::Code {
            language: Some(lang),
        } => lang.as_str(),
        Block::Code { language: None } => "code",
        Block::Text => "text",
    };

    let lines: Vec<Vec<(String, [f32; 4])>> = match block {
        Block::Code { language } => bfmd::highlight_code(language.as_deref(), text)
            .into_iter()
            .map(|line| {
                line.into_iter()
                    .map(|s| (s.text, s.color))
                    .collect::<Vec<_>>()
            })
            .collect(),
        Block::Text => text
            .split('\n')
            .map(|line| vec![(line.to_string(), TEXT_NORMAL)])
            .collect(),
    };

    // Soft-wrap each visual line to inner_w while preserving span colors.
    let mut wrapped: Vec<Vec<(String, [f32; 4])>> = Vec::new();
    for line in &lines {
        wrap_spans(ui, font, size, line, inner_w, &mut wrapped);
    }
    if wrapped.is_empty() {
        wrapped.push(vec![(String::new(), TEXT_NORMAL)]);
    }

    let mut body_h = 0.0f32;
    for line in &wrapped {
        let mut lh = size;
        for (t, _) in line {
            if !t.is_empty() {
                lh = lh.max(skin::text_size(ui, font, size, t)[1].max(size));
            }
        }
        body_h += lh + 2.0;
    }
    body_h = body_h.max(size);

    let total_h = BLOCK_PAD + BLOCK_HEADER_H + body_h + BLOCK_PAD;
    let mut copy = None;

    if let Some(origin) = origin {
        let min = origin;
        let max = [origin[0] + wrap_w, origin[1] + total_h];
        let draw = ui.get_window_draw_list();
        draw.add_rect(min, max, CODE_BLOCK_BG)
            .filled(true)
            .rounding(BLOCK_RADIUS)
            .build();
        draw.add_rect(min, max, CODE_BLOCK_BORDER)
            .filled(false)
            .rounding(BLOCK_RADIUS)
            .thickness(1.0)
            .build();
        // Header strip
        draw.add_rect(
            min,
            [max[0], min[1] + BLOCK_PAD + BLOCK_HEADER_H],
            CODE_BLOCK_HEADER,
        )
        .filled(true)
        .rounding(BLOCK_RADIUS)
        .build();

        skin::text(
            ui,
            fonts.body,
            12.0,
            [min[0] + BLOCK_PAD, min[1] + BLOCK_PAD + 2.0],
            MUTED,
            label,
        );

        // Copy button (top-right).
        let btn_label = "Copy";
        let btn_size = skin::text_size(ui, fonts.body, 12.0, btn_label);
        let btn_w = btn_size[0] + 12.0;
        let btn_h = BLOCK_HEADER_H - 2.0;
        let btn_min = [
            max[0] - BLOCK_PAD - btn_w,
            min[1] + BLOCK_PAD + 1.0,
        ];
        let btn_max = [btn_min[0] + btn_w, btn_min[1] + btn_h];
        ui.set_cursor_screen_pos(btn_min);
        let clicked = {
            let _id = ui.push_id(id as i32);
            ui.invisible_button("bfmd-block-copy", [btn_w, btn_h])
        };
        let hovered = ui.is_item_hovered();
        if hovered {
            draw.add_rect(btn_min, btn_max, TEXT_SECONDARY)
                .filled(true)
                .rounding(4.0)
                .build();
        }
        skin::text(
            ui,
            fonts.body,
            12.0,
            [btn_min[0] + 6.0, btn_min[1] + (btn_h - btn_size[1]) * 0.5],
            if hovered { CODE_BLOCK_BG } else { TEXT_SECONDARY },
            btn_label,
        );
        if clicked {
            copy = Some(text.to_string());
        }

        let mut cy = min[1] + BLOCK_PAD + BLOCK_HEADER_H;
        for line in &wrapped {
            let mut cx = min[0] + BLOCK_PAD;
            let mut lh = size;
            for (t, _) in line {
                if !t.is_empty() {
                    lh = lh.max(skin::text_size(ui, font, size, t)[1].max(size));
                }
            }
            for (t, color) in line {
                if t.is_empty() {
                    continue;
                }
                skin::text(ui, font, size, [cx, cy], *color, t);
                cx += skin::text_size(ui, font, size, t)[0];
            }
            cy += lh + 2.0;
        }
    }

    (total_h, copy)
}

fn wrap_spans(
    ui: &Ui,
    font: Option<FontId>,
    size: f32,
    spans: &[(String, [f32; 4])],
    width: f32,
    out: &mut Vec<Vec<(String, [f32; 4])>>,
) {
    let mut row: Vec<(String, [f32; 4])> = Vec::new();
    let mut row_w = 0.0f32;
    for (text, color) in spans {
        if text.is_empty() {
            continue;
        }
        let mut rest = text.as_str();
        while !rest.is_empty() {
            let mut take = 0usize;
            let mut w = 0.0f32;
            for (i, ch) in rest.char_indices() {
                let cw = skin::text_size(ui, font, size, &ch.to_string())[0];
                if row_w + w + cw > width && (take > 0 || row_w > 0.0) {
                    break;
                }
                take = i + ch.len_utf8();
                w += cw;
            }
            if take == 0 {
                // Single character wider than remaining space — force it.
                let ch = rest.chars().next().unwrap();
                take = ch.len_utf8();
                w = skin::text_size(ui, font, size, &ch.to_string())[0];
            }
            let piece = rest[..take].to_string();
            rest = &rest[take..];
            if row_w > 0.0 && row_w + w > width {
                out.push(std::mem::take(&mut row));
                row_w = 0.0;
            }
            row_w += w;
            if let Some(last) = row.last_mut() {
                if last.1 == *color {
                    last.0.push_str(&piece);
                    continue;
                }
            }
            row.push((piece, *color));
        }
    }
    out.push(row);
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
