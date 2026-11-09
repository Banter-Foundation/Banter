//! ImGui drawing helpers for BFMD runs in chat message rows.

use dear_imgui_rs::{fonts::FontId, Ui};

use crate::bfmd::{self, Run, Style};
use crate::skin;
use crate::theme::{BLURPLE, BLURPLE_TEXT, DIVIDER, TEXT_NORMAL};

const LINE_GAP: f32 = 4.0;
const MENTION_PAD_X: f32 = 4.0;

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

/// Optional game-profile mention paint: resolve display names + click → open profile.
///
/// Extensible later for other mention kinds without changing the Discord token form.
pub struct GameProfilePaint<'a> {
    pub resolve_name: &'a dyn Fn(i64) -> Option<String>,
    /// Set to `(game_id, screen_anchor)` when a mention chip is clicked.
    pub open: &'a mut Option<(i64, [f32; 2])>,
}

/// Height of wrapped BFMD body (no "(edited)" suffix).
pub fn measure(
    ui: &Ui,
    fonts: Fonts,
    source: &str,
    wrap_w: f32,
    resolve_game: Option<&dyn Fn(i64) -> Option<String>>,
) -> f32 {
    let runs = bfmd::parse(source).runs;
    if runs.is_empty() {
        return 0.0;
    }
    layout(ui, fonts, &runs, wrap_w, None, resolve_game, None)
}

/// Draw BFMD body at `origin`; returns height used.
pub fn draw(
    ui: &Ui,
    fonts: Fonts,
    source: &str,
    origin: [f32; 2],
    wrap_w: f32,
    edited_suffix: &str,
    mut games: Option<GameProfilePaint<'_>>,
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
    match games.as_mut() {
        Some(g) => layout(
            ui,
            fonts,
            &runs,
            wrap_w,
            Some(origin),
            Some(g.resolve_name),
            Some(&mut *g.open),
        ),
        None => layout(ui, fonts, &runs, wrap_w, Some(origin), None, None),
    }
}

fn layout(
    ui: &Ui,
    fonts: Fonts,
    runs: &[Run],
    wrap_w: f32,
    origin: Option<[f32; 2]>,
    resolve_game: Option<&dyn Fn(i64) -> Option<String>>,
    mut open: Option<&mut Option<(i64, [f32; 2])>>,
) -> f32 {
    let wrap_w = wrap_w.max(8.0);
    let mut x = 0.0f32;
    let mut y = 0.0f32;
    let mut line_h = 0.0f32;
    let mut saw_hr = false;
    let mut mention_seq = 0u32;

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
        } else if !run.style.hr {
            saw_hr = false;
        }

        let font = fonts.pick(&run.style);
        let size = run.style.size_or_default();
        let color = run.style.color.map(|c| c.to_imgui()).unwrap_or(TEXT_NORMAL);

        for piece in split_game_mentions(&run.text) {
            match piece {
                TextPiece::Plain(text) => {
                    flush_plain_wrapped(
                        ui,
                        font,
                        size,
                        color,
                        &run.style,
                        text,
                        origin,
                        wrap_w,
                        &mut x,
                        &mut y,
                        &mut line_h,
                    );
                }
                TextPiece::Game { id } => {
                    let name = resolve_game.and_then(|resolve| resolve(id));
                    let label = bfmd::game_profile_display_label(id, name.as_deref());
                    let measured = skin::text_size(ui, font, size, &label);
                    let chip_w = measured[0] + MENTION_PAD_X * 2.0;
                    let chip_h = measured[1].max(size);
                    if x > 0.0 && x + chip_w > wrap_w {
                        y += line_h.max(size) + LINE_GAP;
                        x = 0.0;
                        line_h = 0.0;
                    }
                    line_h = line_h.max(chip_h);
                    if let Some(origin) = origin {
                        let pos = [origin[0] + x, origin[1] + y];
                        let max = [pos[0] + chip_w, pos[1] + chip_h];
                        let bg = [BLURPLE[0], BLURPLE[1], BLURPLE[2], 0.28];
                        ui.get_window_draw_list()
                            .add_rect(pos, max, bg)
                            .rounding(3.0)
                            .filled(true)
                            .build();
                        skin::text(
                            ui,
                            font,
                            size,
                            [pos[0] + MENTION_PAD_X, pos[1]],
                            BLURPLE_TEXT,
                            &label,
                        );
                        mention_seq = mention_seq.wrapping_add(1);
                        let hit_id = format!(
                            "bfmd-game-{}-{}-{}",
                            id,
                            mention_seq,
                            ((pos[0] * 10.0) as i32).wrapping_add((pos[1] * 10.0) as i32)
                        );
                        ui.set_cursor_screen_pos(pos);
                        if ui.invisible_button(&hit_id, [chip_w.max(8.0), chip_h.max(8.0)]) {
                            if let Some(slot) = open.as_mut() {
                                **slot = Some((id, [max[0], pos[1]]));
                            }
                        }
                    }
                    x += chip_w;
                }
            }
        }
    }
    y + line_h.max(0.0)
}

enum TextPiece<'a> {
    Plain(&'a str),
    Game { id: i64 },
}

fn split_game_mentions(text: &str) -> Vec<TextPiece<'_>> {
    let hits = bfmd::scan_game_profile_mentions(text);
    if hits.is_empty() {
        return if text.is_empty() {
            Vec::new()
        } else {
            vec![TextPiece::Plain(text)]
        };
    }
    let mut out = Vec::new();
    let mut cursor = 0usize;
    for (start, len, id) in hits {
        if start > cursor {
            out.push(TextPiece::Plain(&text[cursor..start]));
        }
        out.push(TextPiece::Game { id });
        cursor = start + len;
    }
    if cursor < text.len() {
        out.push(TextPiece::Plain(&text[cursor..]));
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn flush_plain_wrapped(
    ui: &Ui,
    font: Option<FontId>,
    size: f32,
    color: [f32; 4],
    style: &Style,
    text: &str,
    origin: Option<[f32; 2]>,
    wrap_w: f32,
    x: &mut f32,
    y: &mut f32,
    line_h: &mut f32,
) {
    let mut pending = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] == '\n' {
            flush_chunk(
                ui, font, size, color, style, &pending, origin, x, y, line_h,
            );
            pending.clear();
            *y += (*line_h).max(size) + LINE_GAP;
            *x = 0.0;
            *line_h = 0.0;
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
        let candidate = format!("{pending}{word}");
        let width = skin::text_size(ui, font, size, &candidate)[0];
        if *x > 0.0
            && pending.is_empty()
            && width > wrap_w - *x
            && !word.chars().all(|c| c.is_whitespace())
        {
            *y += (*line_h).max(size) + LINE_GAP;
            *x = 0.0;
            *line_h = 0.0;
        }
        let width_from_line = skin::text_size(ui, font, size, &format!("{pending}{word}"))[0];
        if *x + width_from_line > wrap_w && !pending.is_empty() {
            flush_chunk(
                ui, font, size, color, style, &pending, origin, x, y, line_h,
            );
            pending.clear();
            *y += (*line_h).max(size) + LINE_GAP;
            *x = 0.0;
            *line_h = 0.0;
        }
        pending.push_str(&word);
    }
    flush_chunk(
        ui, font, size, color, style, &pending, origin, x, y, line_h,
    );
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
