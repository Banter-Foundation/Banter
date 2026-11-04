//! Collaborative drawing board stage (Excalidraw-style shapes on Dear ImGui).

use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

use banter_server::{BoardElement, BoardElementKind, BoardOp, BoardScene, BoardStrokeStyle};
use dear_imgui_rs::{fonts::FontId, Key, ManagedTextureId, MouseButton, Ui};
use image::{Rgba, RgbaImage};

use crate::skin;
use crate::theme::{
    BOARD_CANVAS, BOARD_DEFAULT_FILL, BOARD_DEFAULT_STROKE, BOARD_FILE_CARD, BOARD_GRID, BOARD_GRID_LINE,
    BOARD_HANDLE, BOARD_SELECT, BOARD_STICKY, BOARD_STICKY_TEXT, BOARD_STYLE_ROW_H, BOARD_SWATCH, BOARD_TOOL_GAP,
    BOARD_TOOL_PAD, BOARD_TOOL_ROW_H, BOARD_TOOLBAR_H, DIVIDER, FONT_MD, FONT_SM, FONT_XS, INPUT_BG, MUTED,
    ROW_HOVER, ROW_SELECTED, SIDEBAR_BG, STAGE_BG, TEXT, TEXT_NORMAL, TEXT_SECONDARY, WHITE,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoardTool {
    Select,
    Rect,
    Ellipse,
    Diamond,
    Triangle,
    Line,
    Arrow,
    Freehand,
    Text,
    Sticky,
    Image,
    File,
}

#[derive(Clone, Debug)]
enum Drag {
    Pan {
        last: [f32; 2],
    },
    Create {
        kind: BoardElementKind,
        start: [f64; 2],
        current: [f64; 2],
        points: Vec<[f64; 2]>,
    },
    Move {
        ids: Vec<String>,
        origin: [f64; 2],
        starts: HashMap<String, [f64; 2]>,
    },
    Resize {
        id: String,
        origin: [f64; 2],
        start_box: [f64; 4],
    },
    Marquee {
        start: [f64; 2],
        current: [f64; 2],
    },
}

#[derive(Clone, Debug)]
pub struct BoardTexture {
    pub texture: ManagedTextureId,
    #[allow(dead_code)]
    pub size: [f32; 2],
}

#[derive(Clone, Debug)]
pub struct Brush {
    pub stroke: String,
    pub fill: String,
    pub stroke_width: f64,
    pub stroke_style: BoardStrokeStyle,
}

impl Default for Brush {
    fn default() -> Self {
        Self {
            stroke: BOARD_DEFAULT_STROKE.into(),
            fill: BOARD_DEFAULT_FILL.into(),
            stroke_width: 2.0,
            stroke_style: BoardStrokeStyle::Solid,
        }
    }
}

pub struct BoardState {
    pub channel_id: Option<i64>,
    pub scene: BoardScene,
    pub tool: BoardTool,
    pub brush: Brush,
    pub snap: bool,
    pub pan: [f64; 2],
    pub zoom: f64,
    pub selected: HashSet<String>,
    drag: Option<Drag>,
    undo: Vec<BoardOp>,
    redo: Vec<BoardOp>,
    pub editing_id: Option<String>,
    pub edit_buf: String,
    pub file_preview: Option<(String, String)>,
    pub needed_assets: HashSet<i64>,
    space_pan: bool,
}

impl Default for BoardState {
    fn default() -> Self {
        Self {
            channel_id: None,
            scene: BoardScene::empty(),
            tool: BoardTool::Select,
            brush: Brush::default(),
            snap: false,
            pan: [0.0, 0.0],
            zoom: 1.0,
            selected: HashSet::new(),
            drag: None,
            undo: Vec::new(),
            redo: Vec::new(),
            editing_id: None,
            edit_buf: String::new(),
            file_preview: None,
            needed_assets: HashSet::new(),
            space_pan: false,
        }
    }
}

pub enum BoardCommand {
    Apply(BoardOp),
    PickImage,
    PickFile,
    ExportPng,
}

impl BoardState {
    pub fn reset(&mut self, channel_id: i64, scene: BoardScene) {
        self.channel_id = Some(channel_id);
        self.scene = scene;
        self.selected.clear();
        self.drag = None;
        self.undo.clear();
        self.redo.clear();
        self.editing_id = None;
        self.edit_buf.clear();
        self.file_preview = None;
        self.needed_assets.clear();
        self.pan = [0.0, 0.0];
        self.zoom = 1.0;
        self.note_assets();
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn apply_remote(&mut self, revision: i64, ops: &[BoardOp]) {
        for op in ops {
            let _ = apply_local(&mut self.scene, op.clone());
        }
        self.scene.revision = revision;
        self.note_assets();
    }

    pub fn set_scene(&mut self, scene: BoardScene) {
        self.scene = scene;
        self.note_assets();
    }

    fn note_assets(&mut self) {
        for element in &self.scene.elements {
            if matches!(element.kind, BoardElementKind::Image | BoardElementKind::FileCard) {
                if let Some(id) = element.asset_id {
                    self.needed_assets.insert(id);
                }
            }
        }
    }

    pub fn draw(
        &mut self,
        ui: &Ui,
        bold: Option<FontId>,
        icon: &mut dyn FnMut(&str) -> Option<ManagedTextureId>,
        textures: &HashMap<i64, BoardTexture>,
        user_id: i64,
    ) -> Vec<BoardCommand> {
        let mut commands = Vec::new();
        let origin = ui.cursor_screen_pos();
        let avail = ui.content_region_avail();
        let width = avail[0].max(1.0);
        let height = avail[1].max(1.0);
        let canvas_h = (height - BOARD_TOOLBAR_H).max(1.0);
        let editing = self.editing_id.is_some();

        // Reserve exactly the stage size once — a full dummy plus a canvas button doubled
        // the child height and broke overlay text fields.
        ui.set_cursor_screen_pos(origin);
        ui.dummy([width, BOARD_TOOLBAR_H]);
        let toolbar_max = [origin[0] + width, origin[1] + BOARD_TOOLBAR_H];
        skin::rect(ui, origin, toolbar_max, SIDEBAR_BG, 0.0);
        skin::rect(ui, [origin[0], toolbar_max[1] - 1.0], [origin[0] + width, toolbar_max[1]], DIVIDER, 0.0);
        self.draw_toolbar(ui, bold, icon, origin, width, &mut commands);

        let canvas_min = [origin[0], origin[1] + BOARD_TOOLBAR_H];
        let canvas_max = [origin[0] + width, origin[1] + height];
        skin::rect(ui, canvas_min, canvas_max, BOARD_CANVAS, 0.0);
        self.draw_grid(ui, canvas_min, canvas_max);

        ui.set_cursor_screen_pos(canvas_min);
        let _ = ui.invisible_button("##board-canvas", [width, canvas_h]);
        let hovered = ui.is_item_hovered();
        let focused = !editing && (ui.is_item_active() || hovered);

        self.space_pan = !editing && ui.is_key_down(Key::Space);
        if focused {
            let wheel = ui.io().mouse_wheel();
            if wheel.abs() > 0.0 {
                let before = self.screen_to_world(ui.io().mouse_pos(), canvas_min);
                self.zoom = (self.zoom * (1.0 + wheel as f64 * 0.1)).clamp(0.25, 4.0);
                let after = self.screen_to_world(ui.io().mouse_pos(), canvas_min);
                self.pan[0] += after[0] - before[0];
                self.pan[1] += after[1] - before[1];
            }
        }

        if !editing
            && ((hovered && ui.is_mouse_clicked(MouseButton::Middle))
                || (hovered && self.space_pan && ui.is_mouse_clicked(MouseButton::Left)))
        {
            self.drag = Some(Drag::Pan {
                last: ui.io().mouse_pos(),
            });
        }

        if let Some(Drag::Pan { last }) = self.drag.clone() {
            if ui.is_mouse_down(MouseButton::Middle) || (self.space_pan && ui.is_mouse_down(MouseButton::Left)) {
                let pos = ui.io().mouse_pos();
                self.pan[0] += (pos[0] - last[0]) as f64 / self.zoom;
                self.pan[1] += (pos[1] - last[1]) as f64 / self.zoom;
                self.drag = Some(Drag::Pan { last: pos });
            } else {
                self.drag = None;
            }
        } else if !editing && hovered && !self.space_pan {
            self.handle_pointer(ui, canvas_min, user_id, &mut commands);
        }

        let preview = self.preview_element(user_id);
        for element in self.scene.elements.iter().chain(preview.as_ref()) {
            self.paint_element(ui, bold, element, canvas_min, textures);
        }
        if let Some(Drag::Marquee { start, current }) = &self.drag {
            let a = self.world_to_screen(*start, canvas_min);
            let b = self.world_to_screen(*current, canvas_min);
            let min = [a[0].min(b[0]), a[1].min(b[1])];
            let max = [a[0].max(b[0]), a[1].max(b[1])];
            ui.get_window_draw_list()
                .add_rect(min, max, BOARD_SELECT)
                .thickness(1.0)
                .build();
        }
        for id in &self.selected {
            if let Some(element) = self.scene.elements.iter().find(|item| &item.id == id) {
                self.paint_selection(ui, element, canvas_min);
            }
        }

        if let Some(id) = self.editing_id.clone() {
            if let Some(element) = self.scene.elements.iter().find(|item| item.id == id).cloned() {
                let screen = self.world_to_screen([element.x, element.y], canvas_min);
                let w = (element.w * self.zoom).max(160.0) as f32;
                let h = 36.0_f32;
                // Keep the editor inside the canvas so ImGui does not assert on clipped inputs.
                let x = screen[0].clamp(canvas_min[0] + 4.0, (canvas_max[0] - w - 4.0).max(canvas_min[0] + 4.0));
                let y = screen[1].clamp(canvas_min[1] + 4.0, (canvas_max[1] - h - 4.0).max(canvas_min[1] + 4.0));
                skin::rect(ui, [x, y], [x + w, y + h], INPUT_BG, 4.0);
                let enter = skin::input(
                    ui,
                    "##board-edit",
                    &mut self.edit_buf,
                    "Type here",
                    [x + 4.0, y],
                    (w - 8.0).max(40.0),
                    h,
                    FONT_MD,
                    false,
                );
                if enter {
                    let mut updated = element;
                    updated.text = Some(self.edit_buf.clone());
                    let revision = self.scene.revision;
                    let _ = apply_local(
                        &mut self.scene,
                        BoardOp::Upsert {
                            base_revision: revision,
                            elements: vec![updated.clone()],
                        },
                    );
                    commands.push(BoardCommand::Apply(BoardOp::Upsert {
                        base_revision: revision,
                        elements: vec![updated],
                    }));
                    self.editing_id = None;
                } else if ui.is_key_pressed(Key::Escape) {
                    self.editing_id = None;
                }
            } else {
                self.editing_id = None;
            }
        }

        if let Some((name, body)) = self.file_preview.clone() {
            let pad = 24.0;
            let box_min = [canvas_min[0] + pad, canvas_min[1] + pad];
            let box_max = [canvas_max[0] - pad, canvas_max[1] - pad];
            skin::rect(ui, box_min, box_max, STAGE_BG, 8.0);
            skin::text(ui, bold, FONT_MD, [box_min[0] + 16.0, box_min[1] + 16.0], TEXT, &name);
            let lines = skin::wrap_lines(ui, None, FONT_SM, &body, box_max[0] - box_min[0] - 32.0);
            let mut y = box_min[1] + 48.0;
            for line in lines.into_iter().take(40) {
                skin::text(ui, None, FONT_SM, [box_min[0] + 16.0, y], TEXT_NORMAL, &line);
                y += 18.0;
            }
            if skin::icon_button(
                ui,
                "close-file-preview",
                icon("x"),
                [box_max[0] - 32.0, box_min[1] + 12.0],
                24.0,
                16.0,
                WHITE,
                WHITE,
                None,
            ) || ui.is_key_pressed(Key::Escape)
            {
                self.file_preview = None;
            }
        }

        if focused && ui.is_key_pressed(Key::Delete) && !self.selected.is_empty() {
            let ids: Vec<_> = self.selected.iter().cloned().collect();
            let op = BoardOp::Delete {
                base_revision: self.scene.revision,
                ids,
            };
            commands.push(BoardCommand::Apply(op));
            self.selected.clear();
        }
        if focused && ui.is_key_down(Key::ModCtrl) && ui.is_key_pressed(Key::Z) {
            if let Some(op) = self.undo.pop() {
                self.redo.push(op);
            }
        }

        commands
    }

    fn draw_toolbar(
        &mut self,
        ui: &Ui,
        bold: Option<FontId>,
        icon: &mut dyn FnMut(&str) -> Option<ManagedTextureId>,
        origin: [f32; 2],
        width: f32,
        commands: &mut Vec<BoardCommand>,
    ) {
        let tools = [
            (BoardTool::Select, "mouse-pointer-2", "Select"),
            (BoardTool::Rect, "square", "Rect"),
            (BoardTool::Ellipse, "circle", "Ellipse"),
            (BoardTool::Diamond, "square", "Diamond"),
            (BoardTool::Triangle, "square", "Triangle"),
            (BoardTool::Line, "minus", "Line"),
            (BoardTool::Arrow, "minus", "Arrow"),
            (BoardTool::Freehand, "pencil", "Draw"),
            (BoardTool::Text, "type", "Text"),
            (BoardTool::Sticky, "sticky-note", "Sticky"),
            (BoardTool::Image, "image", "Image"),
            (BoardTool::File, "file-text", "File"),
        ];
        let mut x = origin[0] + BOARD_TOOL_PAD;
        let y = origin[1] + (BOARD_TOOL_ROW_H - 28.0) * 0.5;
        for (tool, name, tip) in tools {
            let chosen = self.tool == tool;
            let fill = if chosen { ROW_SELECTED } else { SIDEBAR_BG };
            let min = [x, y];
            let max = [x + 28.0, y + 28.0];
            ui.set_cursor_screen_pos(min);
            if ui.invisible_button(format!("board-tool-{tip}"), [28.0, 28.0]) {
                self.tool = tool;
                if tool == BoardTool::Image {
                    commands.push(BoardCommand::PickImage);
                } else if tool == BoardTool::File {
                    commands.push(BoardCommand::PickFile);
                }
            }
            let hovered = ui.is_item_hovered();
            skin::rect(ui, min, max, if hovered && !chosen { ROW_HOVER } else { fill }, 4.0);
            // Diamond / triangle share the square glyph; label them so they stay distinct.
            if tip == "Diamond" {
                skin::text_centered(ui, bold, FONT_SM, min[0] + 14.0, min[1] + 5.0, if chosen { WHITE } else { TEXT_SECONDARY }, "◇");
            } else if tip == "Triangle" {
                skin::text_centered(ui, bold, FONT_SM, min[0] + 14.0, min[1] + 5.0, if chosen { WHITE } else { TEXT_SECONDARY }, "△");
            } else {
                skin::image(ui, icon(name), [min[0] + 6.0, min[1] + 6.0], 16.0, if chosen { WHITE } else { TEXT_SECONDARY });
            }
            if hovered {
                skin::tooltip(ui, bold, FONT_XS, tip, min, max, crate::theme::TooltipSide::Top);
            }
            x += 28.0 + BOARD_TOOL_GAP;
        }
        x += 8.0;
        let snap_min = [x, y];
        ui.set_cursor_screen_pos(snap_min);
        if ui.invisible_button("board-snap", [56.0, 28.0]) {
            self.snap = !self.snap;
        }
        let snap_fill = if self.snap { ROW_SELECTED } else if ui.is_item_hovered() { ROW_HOVER } else { SIDEBAR_BG };
        skin::rect(ui, snap_min, [snap_min[0] + 56.0, snap_min[1] + 28.0], snap_fill, 4.0);
        skin::text_centered(ui, bold, FONT_XS, snap_min[0] + 28.0, snap_min[1] + 7.0, TEXT_NORMAL, "Snap");
        x += 64.0;
        if !self.selected.is_empty() {
            ui.set_cursor_screen_pos([x, y]);
            if ui.invisible_button("board-group", [56.0, 28.0]) {
                let group_id = new_id("g");
                let ids: Vec<_> = self.selected.iter().cloned().collect();
                commands.push(BoardCommand::Apply(BoardOp::Group {
                    base_revision: self.scene.revision,
                    ids,
                    group_id,
                }));
            }
            let hovered = ui.is_item_hovered();
            skin::rect(ui, [x, y], [x + 56.0, y + 28.0], if hovered { ROW_HOVER } else { SIDEBAR_BG }, 4.0);
            skin::text_centered(ui, bold, FONT_XS, x + 28.0, y + 7.0, TEXT_NORMAL, "Group");
            x += 64.0;
            ui.set_cursor_screen_pos([x, y]);
            if ui.invisible_button("board-ungroup", [72.0, 28.0]) {
                let group_ids: Vec<_> = self
                    .scene
                    .elements
                    .iter()
                    .filter(|element| self.selected.contains(&element.id))
                    .filter_map(|element| element.group_id.clone())
                    .collect::<HashSet<_>>()
                    .into_iter()
                    .collect();
                if !group_ids.is_empty() {
                    commands.push(BoardCommand::Apply(BoardOp::Ungroup {
                        base_revision: self.scene.revision,
                        group_ids,
                    }));
                }
            }
            let hovered = ui.is_item_hovered();
            skin::rect(ui, [x, y], [x + 72.0, y + 28.0], if hovered { ROW_HOVER } else { SIDEBAR_BG }, 4.0);
            skin::text_centered(ui, bold, FONT_XS, x + 36.0, y + 7.0, TEXT_NORMAL, "Ungroup");
        }
        let export_x = origin[0] + width - BOARD_TOOL_PAD - 72.0;
        ui.set_cursor_screen_pos([export_x, y]);
        if ui.invisible_button("board-export", [72.0, 28.0]) {
            commands.push(BoardCommand::ExportPng);
        }
        let hovered = ui.is_item_hovered();
        skin::rect(ui, [export_x, y], [export_x + 72.0, y + 28.0], if hovered { ROW_HOVER } else { SIDEBAR_BG }, 4.0);
        skin::text_centered(ui, bold, FONT_XS, export_x + 36.0, y + 7.0, TEXT_NORMAL, "Export");

        // Style row: stroke color, fill color, width, line style.
        let style_y = origin[1] + BOARD_TOOL_ROW_H + (BOARD_STYLE_ROW_H - BOARD_SWATCH) * 0.5;
        skin::rect(
            ui,
            [origin[0], origin[1] + BOARD_TOOL_ROW_H],
            [origin[0] + width, origin[1] + BOARD_TOOLBAR_H],
            SIDEBAR_BG,
            0.0,
        );
        skin::rect(
            ui,
            [origin[0], origin[1] + BOARD_TOOL_ROW_H],
            [origin[0] + width, origin[1] + BOARD_TOOL_ROW_H + 1.0],
            DIVIDER,
            0.0,
        );
        let mut sx = origin[0] + BOARD_TOOL_PAD;
        skin::text(ui, bold, FONT_XS, [sx, style_y + 2.0], MUTED, "Stroke");
        sx += 44.0;
        for (i, hex) in STROKE_SWATCHES.iter().enumerate() {
            swatch_button(ui, &format!("stroke-{i}"), hex, &mut self.brush.stroke, sx, style_y, false);
            sx += BOARD_SWATCH + 4.0;
        }
        sx += 12.0;
        skin::text(ui, bold, FONT_XS, [sx, style_y + 2.0], MUTED, "Fill");
        sx += 28.0;
        swatch_button(ui, "fill-none", "#00000000", &mut self.brush.fill, sx, style_y, true);
        sx += BOARD_SWATCH + 4.0;
        for (i, hex) in FILL_SWATCHES.iter().enumerate() {
            swatch_button(ui, &format!("fill-{i}"), hex, &mut self.brush.fill, sx, style_y, false);
            sx += BOARD_SWATCH + 4.0;
        }
        sx += 12.0;
        skin::text(ui, bold, FONT_XS, [sx, style_y + 2.0], MUTED, "Width");
        sx += 40.0;
        for (label, width) in [("S", 1.0), ("M", 2.0), ("L", 4.0), ("XL", 8.0)] {
            let chosen = (self.brush.stroke_width - width).abs() < 0.1;
            let min = [sx, style_y - 2.0];
            ui.set_cursor_screen_pos(min);
            if ui.invisible_button(format!("width-{label}"), [28.0, 22.0]) {
                self.brush.stroke_width = width;
            }
            let hovered = ui.is_item_hovered();
            skin::rect(
                ui,
                min,
                [min[0] + 28.0, min[1] + 22.0],
                if chosen {
                    ROW_SELECTED
                } else if hovered {
                    ROW_HOVER
                } else {
                    SIDEBAR_BG
                },
                4.0,
            );
            skin::text_centered(ui, bold, FONT_XS, min[0] + 14.0, min[1] + 3.0, TEXT_NORMAL, label);
            sx += 32.0;
        }
        sx += 8.0;
        skin::text(ui, bold, FONT_XS, [sx, style_y + 2.0], MUTED, "Style");
        sx += 36.0;
        for (label, style) in [
            ("Solid", BoardStrokeStyle::Solid),
            ("Dash", BoardStrokeStyle::Dashed),
            ("Dot", BoardStrokeStyle::Dotted),
        ] {
            let chosen = self.brush.stroke_style == style;
            let min = [sx, style_y - 2.0];
            ui.set_cursor_screen_pos(min);
            if ui.invisible_button(format!("style-{label}"), [40.0, 22.0]) {
                self.brush.stroke_style = style;
            }
            let hovered = ui.is_item_hovered();
            skin::rect(
                ui,
                min,
                [min[0] + 40.0, min[1] + 22.0],
                if chosen {
                    ROW_SELECTED
                } else if hovered {
                    ROW_HOVER
                } else {
                    SIDEBAR_BG
                },
                4.0,
            );
            skin::text_centered(ui, bold, FONT_XS, min[0] + 20.0, min[1] + 3.0, TEXT_NORMAL, label);
            sx += 44.0;
        }
    }


    fn draw_grid(&self, ui: &Ui, canvas_min: [f32; 2], canvas_max: [f32; 2]) {
        if self.zoom < 0.4 {
            return;
        }
        let draw = ui.get_window_draw_list();
        let step = (BOARD_GRID * self.zoom) as f32;
        if step < 8.0 {
            return;
        }
        let ox = ((self.pan[0] * self.zoom) as f32).rem_euclid(step);
        let oy = ((self.pan[1] * self.zoom) as f32).rem_euclid(step);
        let mut x = canvas_min[0] + ox;
        while x < canvas_max[0] {
            draw.add_line([x, canvas_min[1]], [x, canvas_max[1]], BOARD_GRID_LINE)
                .thickness(1.0)
                .build();
            x += step;
        }
        let mut y = canvas_min[1] + oy;
        while y < canvas_max[1] {
            draw.add_line([canvas_min[0], y], [canvas_max[0], y], BOARD_GRID_LINE)
                .thickness(1.0)
                .build();
            y += step;
        }
    }

    fn handle_pointer(&mut self, ui: &Ui, canvas_min: [f32; 2], user_id: i64, commands: &mut Vec<BoardCommand>) {
        let mouse = ui.io().mouse_pos();
        let raw = self.screen_to_world(mouse, canvas_min);
        // Freehand stays free; snap only applies to shape endpoints / moves when enabled.
        let world = if matches!(self.tool, BoardTool::Freehand)
            || matches!(
                &self.drag,
                Some(Drag::Create {
                    kind: BoardElementKind::Freehand,
                    ..
                })
            ) {
            raw
        } else {
            self.snap_point(raw)
        };

        if let Some(drag) = self.drag.clone() {
            match drag {
                Drag::Create {
                    kind,
                    start,
                    mut points,
                    ..
                } => {
                    if ui.is_mouse_down(MouseButton::Left) {
                        if kind == BoardElementKind::Freehand {
                            if points.last().map(|p| (p[0] - world[0]).hypot(p[1] - world[1]) > 1.0).unwrap_or(true) {
                                points.push(world);
                            }
                        }
                        self.drag = Some(Drag::Create {
                            kind,
                            start,
                            current: world,
                            points,
                        });
                    } else {
                        let end = if kind == BoardElementKind::Freehand {
                            world
                        } else {
                            self.snap_point(world)
                        };
                        if let Some(element) =
                            finish_create(kind, start, end, points, user_id, &self.scene, &self.brush)
                        {
                            let op = BoardOp::Upsert {
                                base_revision: self.scene.revision,
                                elements: vec![element.clone()],
                            };
                            let revision = self.scene.revision;
                            let _ = apply_local(&mut self.scene, op.clone());
                            // apply_local does not bump revision; keep client revision until server ack
                            self.scene.revision = revision;
                            self.selected.clear();
                            self.selected.insert(element.id.clone());
                            if matches!(kind, BoardElementKind::Text | BoardElementKind::Sticky) {
                                self.editing_id = Some(element.id.clone());
                                self.edit_buf = element.text.clone().unwrap_or_default();
                                self.tool = BoardTool::Select;
                            }
                            self.push_undo(op.clone());
                            commands.push(BoardCommand::Apply(op));
                        }
                        self.drag = None;
                    }
                }
                Drag::Move { ids, origin, starts } => {
                    if ui.is_mouse_down(MouseButton::Left) {
                        let dx = world[0] - origin[0];
                        let dy = world[1] - origin[1];
                        let mut elements = Vec::new();
                        for id in &ids {
                            if let Some(mut element) = self.scene.elements.iter().find(|item| &item.id == id).cloned() {
                                if let Some(start) = starts.get(id) {
                                    element.x = start[0] + dx;
                                    element.y = start[1] + dy;
                                    if self.snap {
                                        element.x = snap(element.x);
                                        element.y = snap(element.y);
                                    }
                                    elements.push(element);
                                }
                            }
                        }
                        if !elements.is_empty() {
                            let revision = self.scene.revision;
                            let _ = apply_local(
                                &mut self.scene,
                                BoardOp::Upsert {
                                    base_revision: revision,
                                    elements: elements.clone(),
                                },
                            );
                            self.drag = Some(Drag::Move { ids, origin, starts });
                        }
                    } else {
                        let elements: Vec<_> = ids
                            .iter()
                            .filter_map(|id| self.scene.elements.iter().find(|item| &item.id == id).cloned())
                            .collect();
                        if !elements.is_empty() {
                            let op = BoardOp::Upsert {
                                base_revision: self.scene.revision,
                                elements,
                            };
                            self.push_undo(op.clone());
                            commands.push(BoardCommand::Apply(op));
                        }
                        self.drag = None;
                    }
                }
                Drag::Resize { id, origin, start_box } => {
                    if ui.is_mouse_down(MouseButton::Left) {
                        if let Some(mut element) = self.scene.elements.iter().find(|item| item.id == id).cloned() {
                            let dx = world[0] - origin[0];
                            let dy = world[1] - origin[1];
                            element.w = (start_box[2] + dx).max(8.0);
                            element.h = (start_box[3] + dy).max(8.0);
                            if self.snap {
                                element.w = snap(element.w);
                                element.h = snap(element.h);
                            }
                            let revision = self.scene.revision;
                            let _ = apply_local(
                                &mut self.scene,
                                BoardOp::Upsert {
                                    base_revision: revision,
                                    elements: vec![element],
                                },
                            );
                            self.drag = Some(Drag::Resize { id, origin, start_box });
                        }
                    } else {
                        if let Some(element) = self.scene.elements.iter().find(|item| item.id == id).cloned() {
                            let op = BoardOp::Upsert {
                                base_revision: self.scene.revision,
                                elements: vec![element],
                            };
                            self.push_undo(op.clone());
                            commands.push(BoardCommand::Apply(op));
                        }
                        self.drag = None;
                    }
                }
                Drag::Marquee { start, .. } => {
                    if ui.is_mouse_down(MouseButton::Left) {
                        self.drag = Some(Drag::Marquee { start, current: world });
                    } else {
                        let min_x = start[0].min(world[0]);
                        let max_x = start[0].max(world[0]);
                        let min_y = start[1].min(world[1]);
                        let max_y = start[1].max(world[1]);
                        if !ui.is_key_down(Key::ModShift) {
                            self.selected.clear();
                        }
                        for element in &self.scene.elements {
                            let ex1 = element.x + element.w;
                            let ey1 = element.y + element.h;
                            if element.x < max_x && ex1 > min_x && element.y < max_y && ey1 > min_y {
                                self.selected.insert(element.id.clone());
                            }
                        }
                        self.drag = None;
                    }
                }
                Drag::Pan { .. } => {}
            }
            return;
        }

        if ui.is_mouse_double_clicked(MouseButton::Left) {
            if let Some(id) = self.hit_test(world) {
                if let Some(element) = self.scene.elements.iter().find(|item| item.id == id) {
                    match element.kind {
                        BoardElementKind::Text | BoardElementKind::Sticky => {
                            self.editing_id = Some(id);
                            self.edit_buf = element.text.clone().unwrap_or_default();
                        }
                        BoardElementKind::FileCard => {
                            let name = element.filename.clone().unwrap_or_else(|| "file".into());
                            let body = element.text.clone().unwrap_or_else(|| "(empty file)".into());
                            self.file_preview = Some((name, body));
                        }
                        _ => {}
                    }
                }
            }
            return;
        }

        if !ui.is_mouse_clicked(MouseButton::Left) {
            return;
        }

        match self.tool {
            BoardTool::Select => {
                if let Some(id) = self.hit_test(world) {
                    if let Some(element) = self.scene.elements.iter().find(|item| item.id == id) {
                        let handle = [
                            element.x + element.w,
                            element.y + element.h,
                        ];
                        if (world[0] - handle[0]).hypot(world[1] - handle[1]) < 10.0 / self.zoom {
                            self.drag = Some(Drag::Resize {
                                id: id.clone(),
                                origin: world,
                                start_box: [element.x, element.y, element.w, element.h],
                            });
                            return;
                        }
                    }
                    if !ui.is_key_down(Key::ModShift) && !self.selected.contains(&id) {
                        self.selected.clear();
                    }
                    // expand group selection
                    if let Some(group) = self
                        .scene
                        .elements
                        .iter()
                        .find(|item| item.id == id)
                        .and_then(|item| item.group_id.clone())
                    {
                        for element in &self.scene.elements {
                            if element.group_id.as_ref() == Some(&group) {
                                self.selected.insert(element.id.clone());
                            }
                        }
                    } else {
                        self.selected.insert(id);
                    }
                    let ids: Vec<_> = self.selected.iter().cloned().collect();
                    let starts = ids
                        .iter()
                        .filter_map(|id| {
                            self.scene
                                .elements
                                .iter()
                                .find(|item| &item.id == id)
                                .map(|element| (id.clone(), [element.x, element.y]))
                        })
                        .collect();
                    self.drag = Some(Drag::Move {
                        ids,
                        origin: world,
                        starts,
                    });
                } else {
                    self.drag = Some(Drag::Marquee {
                        start: world,
                        current: world,
                    });
                }
            }
            BoardTool::Image | BoardTool::File => {}
            other => {
                let kind = tool_kind(other);
                self.drag = Some(Drag::Create {
                    kind,
                    start: world,
                    current: world,
                    points: vec![world],
                });
            }
        }
    }

    fn preview_element(&self, user_id: i64) -> Option<BoardElement> {
        match &self.drag {
            Some(Drag::Create {
                kind,
                start,
                current,
                points,
            }) => finish_create(*kind, *start, *current, points.clone(), user_id, &self.scene, &self.brush),
            _ => None,
        }
    }

    fn paint_element(
        &self,
        ui: &Ui,
        bold: Option<FontId>,
        element: &BoardElement,
        canvas_min: [f32; 2],
        textures: &HashMap<i64, BoardTexture>,
    ) {
        let left = element.x.min(element.x + element.w);
        let top = element.y.min(element.y + element.h);
        let right = element.x.max(element.x + element.w);
        let bottom = element.y.max(element.y + element.h);
        let min = self.world_to_screen([left, top], canvas_min);
        let max = self.world_to_screen([right, bottom], canvas_min);
        // Lines keep signed endpoints for direction.
        let line_a = self.world_to_screen([element.x, element.y], canvas_min);
        let line_b = self.world_to_screen([element.x + element.w, element.y + element.h], canvas_min);
        let stroke = parse_color(&element.stroke).unwrap_or(TEXT_NORMAL);
        let fill = parse_color(&element.fill).unwrap_or([0.0, 0.0, 0.0, 0.0]);
        let width = stroke_px(element.stroke_width);
        let style = element.stroke_style;
        match element.kind {
            BoardElementKind::Rect | BoardElementKind::Sticky => {
                {
                    let draw = ui.get_window_draw_list();
                    let color = if element.kind == BoardElementKind::Sticky { BOARD_STICKY } else { fill };
                    if color[3] > 0.05 {
                        draw.add_rect(min, max, color).rounding(4.0).filled(true).build();
                    }
                }
                draw_rect_stroke(ui, min, max, stroke, width, style);
                if let Some(text) = &element.text {
                    let color = if element.kind == BoardElementKind::Sticky {
                        BOARD_STICKY_TEXT
                    } else {
                        TEXT
                    };
                    let lines = skin::wrap_lines(ui, bold, FONT_SM, text, (max[0] - min[0] - 12.0).max(8.0));
                    let mut y = min[1] + 8.0;
                    for line in lines {
                        skin::text(ui, bold, FONT_SM, [min[0] + 8.0, y], color, &line);
                        y += 18.0;
                        if y > max[1] - 8.0 {
                            break;
                        }
                    }
                }
            }
            BoardElementKind::Ellipse => {
                let draw = ui.get_window_draw_list();
                let center = [(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5];
                let rx = ((max[0] - min[0]).abs() * 0.5).max(1.0);
                let ry = ((max[1] - min[1]).abs() * 0.5).max(1.0);
                let radius = rx.min(ry);
                if fill[3] > 0.05 {
                    draw.add_circle(center, radius, fill).filled(true).build();
                }
                draw.add_circle(center, radius, stroke).thickness(width).build();
            }
            BoardElementKind::Diamond => {
                let cx = (min[0] + max[0]) * 0.5;
                let cy = (min[1] + max[1]) * 0.5;
                let pts = [[cx, min[1]], [max[0], cy], [cx, max[1]], [min[0], cy]];
                paint_poly(ui, &pts, fill, stroke, width, style);
            }
            BoardElementKind::Triangle => {
                let cx = (min[0] + max[0]) * 0.5;
                let pts = [[cx, min[1]], [max[0], max[1]], [min[0], max[1]]];
                paint_poly(ui, &pts, fill, stroke, width, style);
            }
            BoardElementKind::Line | BoardElementKind::Arrow => {
                draw_styled_line(ui, line_a, line_b, stroke, width, style);
                if element.kind == BoardElementKind::Arrow {
                    let angle = (line_b[1] - line_a[1]).atan2(line_b[0] - line_a[0]);
                    let len = (10.0 + width * 2.0).min(24.0);
                    let a = [
                        line_b[0] - len * (angle + 0.4).cos(),
                        line_b[1] - len * (angle + 0.4).sin(),
                    ];
                    let b = [
                        line_b[0] - len * (angle - 0.4).cos(),
                        line_b[1] - len * (angle - 0.4).sin(),
                    ];
                    draw_styled_line(ui, line_b, a, stroke, width, BoardStrokeStyle::Solid);
                    draw_styled_line(ui, line_b, b, stroke, width, BoardStrokeStyle::Solid);
                }
            }
            BoardElementKind::Freehand => {
                let pts: Vec<[f32; 2]> = element
                    .points
                    .iter()
                    .map(|p| self.world_to_screen(*p, canvas_min))
                    .collect();
                for window in pts.windows(2) {
                    draw_styled_line(ui, window[0], window[1], stroke, width, style);
                }
            }
            BoardElementKind::Text => {
                if let Some(text) = &element.text {
                    skin::text(ui, bold, FONT_MD, min, stroke, text);
                } else {
                    skin::text(ui, bold, FONT_MD, min, MUTED, "Text");
                }
            }
            BoardElementKind::Image => {
                let loading = if let Some(asset_id) = element.asset_id {
                    if let Some(shot) = textures.get(&asset_id) {
                        ui.get_window_draw_list()
                            .add_image(shot.texture, min, max, [0.0, 0.0], [1.0, 1.0], WHITE);
                        false
                    } else {
                        ui.get_window_draw_list()
                            .add_rect(min, max, ROW_HOVER)
                            .filled(true)
                            .build();
                        true
                    }
                } else {
                    false
                };
                if loading {
                    skin::text(ui, None, FONT_SM, [min[0] + 8.0, min[1] + 8.0], MUTED, "Loading…");
                }
            }
            BoardElementKind::FileCard => {
                {
                    let draw = ui.get_window_draw_list();
                    draw.add_rect(min, max, BOARD_FILE_CARD).rounding(6.0).filled(true).build();
                    draw.add_rect(min, max, stroke).rounding(6.0).thickness(width.max(1.0)).build();
                }
                let name = element.filename.as_deref().unwrap_or("file");
                skin::text(ui, bold, FONT_SM, [min[0] + 12.0, min[1] + 12.0], TEXT, name);
                if let Some(preview) = element.text.as_deref() {
                    let line = preview.lines().next().unwrap_or("");
                    skin::text(ui, None, FONT_XS, [min[0] + 12.0, min[1] + 34.0], TEXT_SECONDARY, line);
                }
            }
        }
    }

    fn paint_selection(&self, ui: &Ui, element: &BoardElement, canvas_min: [f32; 2]) {
        let left = element.x.min(element.x + element.w);
        let top = element.y.min(element.y + element.h);
        let right = element.x.max(element.x + element.w);
        let bottom = element.y.max(element.y + element.h);
        let min = self.world_to_screen([left, top], canvas_min);
        let max = self.world_to_screen([right, bottom], canvas_min);
        let draw = ui.get_window_draw_list();
        draw.add_rect(min, max, BOARD_SELECT).thickness(1.5).build();
        let handle = BOARD_HANDLE;
        let hx = max[0] - handle * 0.5;
        let hy = max[1] - handle * 0.5;
        draw.add_rect([hx, hy], [hx + handle, hy + handle], BOARD_SELECT)
            .filled(true)
            .build();
    }

    fn hit_test(&self, world: [f64; 2]) -> Option<String> {
        for element in self.scene.elements.iter().rev() {
            if element.locked {
                continue;
            }
            if matches!(element.kind, BoardElementKind::Freehand) {
                for point in &element.points {
                    if (point[0] - world[0]).hypot(point[1] - world[1]) < 8.0 / self.zoom {
                        return Some(element.id.clone());
                    }
                }
                continue;
            }
            if world[0] >= element.x
                && world[0] <= element.x + element.w.max(4.0)
                && world[1] >= element.y
                && world[1] <= element.y + element.h.max(4.0)
            {
                return Some(element.id.clone());
            }
        }
        None
    }

    fn screen_to_world(&self, screen: [f32; 2], canvas_min: [f32; 2]) -> [f64; 2] {
        [
            (screen[0] - canvas_min[0]) as f64 / self.zoom - self.pan[0],
            (screen[1] - canvas_min[1]) as f64 / self.zoom - self.pan[1],
        ]
    }

    fn world_to_screen(&self, world: [f64; 2], canvas_min: [f32; 2]) -> [f32; 2] {
        [
            canvas_min[0] + ((world[0] + self.pan[0]) * self.zoom) as f32,
            canvas_min[1] + ((world[1] + self.pan[1]) * self.zoom) as f32,
        ]
    }

    fn snap_point(&self, point: [f64; 2]) -> [f64; 2] {
        if self.snap {
            [snap(point[0]), snap(point[1])]
        } else {
            point
        }
    }

    fn push_undo(&mut self, op: BoardOp) {
        self.undo.push(op);
        self.redo.clear();
        if self.undo.len() > 100 {
            self.undo.remove(0);
        }
    }

    pub fn export_png(&self) -> Option<RgbaImage> {
        if self.scene.elements.is_empty() {
            return None;
        }
        let mut min_x = f64::MAX;
        let mut min_y = f64::MAX;
        let mut max_x = f64::MIN;
        let mut max_y = f64::MIN;
        for element in &self.scene.elements {
            min_x = min_x.min(element.x);
            min_y = min_y.min(element.y);
            max_x = max_x.max(element.x + element.w.max(1.0));
            max_y = max_y.max(element.y + element.h.max(1.0));
            for point in &element.points {
                min_x = min_x.min(point[0]);
                min_y = min_y.min(point[1]);
                max_x = max_x.max(point[0]);
                max_y = max_y.max(point[1]);
            }
        }
        let pad = 24.0;
        min_x -= pad;
        min_y -= pad;
        max_x += pad;
        max_y += pad;
        let width = ((max_x - min_x).ceil() as u32).clamp(64, 4096);
        let height = ((max_y - min_y).ceil() as u32).clamp(64, 4096);
        let mut image = RgbaImage::from_pixel(width, height, Rgba([30, 31, 34, 255]));
        for element in &self.scene.elements {
            let color = parse_color_u8(&element.stroke).unwrap_or([219, 222, 225, 255]);
            let fill = parse_color_u8(&element.fill).unwrap_or([0, 0, 0, 0]);
            let x0 = ((element.x - min_x).round() as i32).max(0) as u32;
            let y0 = ((element.y - min_y).round() as i32).max(0) as u32;
            let x1 = ((element.x + element.w - min_x).round() as i32).clamp(0, width as i32 - 1) as u32;
            let y1 = ((element.y + element.h - min_y).round() as i32).clamp(0, height as i32 - 1) as u32;
            match element.kind {
                BoardElementKind::Rect | BoardElementKind::Sticky | BoardElementKind::FileCard | BoardElementKind::Image => {
                    let fill_c = if element.kind == BoardElementKind::Sticky {
                        [254, 231, 92, 255]
                    } else {
                        fill
                    };
                    fill_rect(&mut image, x0, y0, x1, y1, fill_c);
                    stroke_rect(&mut image, x0, y0, x1, y1, color);
                }
                BoardElementKind::Ellipse => {
                    fill_ellipse(&mut image, x0, y0, x1, y1, fill);
                    stroke_ellipse(&mut image, x0, y0, x1, y1, color);
                }
                BoardElementKind::Line | BoardElementKind::Arrow => {
                    draw_line(&mut image, x0 as i32, y0 as i32, x1 as i32, y1 as i32, color);
                }
                BoardElementKind::Freehand => {
                    let pts: Vec<_> = element
                        .points
                        .iter()
                        .map(|p| (((p[0] - min_x) as i32), ((p[1] - min_y) as i32)))
                        .collect();
                    for window in pts.windows(2) {
                        draw_line(&mut image, window[0].0, window[0].1, window[1].0, window[1].1, color);
                    }
                }
                BoardElementKind::Diamond | BoardElementKind::Triangle | BoardElementKind::Text => {
                    fill_rect(&mut image, x0, y0, x1, y1, fill);
                    stroke_rect(&mut image, x0, y0, x1, y1, color);
                }
            }
        }
        Some(image)
    }

    pub fn place_image_element(&mut self, user_id: i64, asset_id: i64, filename: &str, w: f64, h: f64) -> BoardOp {
        let element = BoardElement {
            id: new_id("e"),
            kind: BoardElementKind::Image,
            x: -self.pan[0] + 80.0,
            y: -self.pan[1] + 80.0,
            w,
            h,
            points: Vec::new(),
            stroke: self.brush.stroke.clone(),
            fill: "#00000000".into(),
            stroke_width: self.brush.stroke_width.max(1.0),
            stroke_style: self.brush.stroke_style,
            rotation: 0.0,
            z: next_z(&self.scene),
            locked: false,
            group_id: None,
            text: None,
            asset_id: Some(asset_id),
            filename: Some(filename.to_string()),
            updated_by: user_id,
        };
        BoardOp::Upsert {
            base_revision: self.scene.revision,
            elements: vec![element],
        }
    }

    pub fn place_file_element(&mut self, user_id: i64, asset_id: i64, filename: &str, preview: &str) -> BoardOp {
        let element = BoardElement {
            id: new_id("e"),
            kind: BoardElementKind::FileCard,
            x: -self.pan[0] + 80.0,
            y: -self.pan[1] + 80.0,
            w: 220.0,
            h: 96.0,
            points: Vec::new(),
            stroke: self.brush.stroke.clone(),
            fill: "#2B2D31FF".into(),
            stroke_width: self.brush.stroke_width.max(1.0),
            stroke_style: self.brush.stroke_style,
            rotation: 0.0,
            z: next_z(&self.scene),
            locked: false,
            group_id: None,
            text: Some(preview.chars().take(400).collect()),
            asset_id: Some(asset_id),
            filename: Some(filename.to_string()),
            updated_by: user_id,
        };
        BoardOp::Upsert {
            base_revision: self.scene.revision,
            elements: vec![element],
        }
    }
}

fn tool_kind(tool: BoardTool) -> BoardElementKind {
    match tool {
        BoardTool::Rect => BoardElementKind::Rect,
        BoardTool::Ellipse => BoardElementKind::Ellipse,
        BoardTool::Diamond => BoardElementKind::Diamond,
        BoardTool::Triangle => BoardElementKind::Triangle,
        BoardTool::Line => BoardElementKind::Line,
        BoardTool::Arrow => BoardElementKind::Arrow,
        BoardTool::Freehand => BoardElementKind::Freehand,
        BoardTool::Text => BoardElementKind::Text,
        BoardTool::Sticky => BoardElementKind::Sticky,
        BoardTool::Select | BoardTool::Image | BoardTool::File => BoardElementKind::Rect,
    }
}

fn finish_create(
    kind: BoardElementKind,
    start: [f64; 2],
    end: [f64; 2],
    points: Vec<[f64; 2]>,
    user_id: i64,
    scene: &BoardScene,
    brush: &Brush,
) -> Option<BoardElement> {
    let (x, y, w, h, points) = match kind {
        BoardElementKind::Freehand => {
            if points.len() < 2 {
                return None;
            }
            let min_x = points.iter().map(|p| p[0]).fold(f64::MAX, f64::min);
            let min_y = points.iter().map(|p| p[1]).fold(f64::MAX, f64::min);
            let max_x = points.iter().map(|p| p[0]).fold(f64::MIN, f64::max);
            let max_y = points.iter().map(|p| p[1]).fold(f64::MIN, f64::max);
            (min_x, min_y, (max_x - min_x).max(1.0), (max_y - min_y).max(1.0), points)
        }
        BoardElementKind::Text | BoardElementKind::Sticky => {
            let w = if kind == BoardElementKind::Sticky { 180.0 } else { 160.0 };
            let h = if kind == BoardElementKind::Sticky { 120.0 } else { 36.0 };
            (start[0], start[1], w, h, Vec::new())
        }
        BoardElementKind::Line | BoardElementKind::Arrow => {
            (start[0], start[1], end[0] - start[0], end[1] - start[1], vec![start, end])
        }
        _ => {
            let x = start[0].min(end[0]);
            let y = start[1].min(end[1]);
            let w = (end[0] - start[0]).abs();
            let h = (end[1] - start[1]).abs();
            if w < 2.0 && h < 2.0 {
                return None;
            }
            (x, y, w.max(2.0), h.max(2.0), Vec::new())
        }
    };
    let fill = if kind == BoardElementKind::Sticky {
        "#FEE75CFF".into()
    } else if matches!(
        kind,
        BoardElementKind::Rect | BoardElementKind::Ellipse | BoardElementKind::Diamond | BoardElementKind::Triangle
    ) {
        brush.fill.clone()
    } else {
        "#00000000".into()
    };
    Some(BoardElement {
        id: new_id("e"),
        kind,
        x,
        y,
        w,
        h,
        points,
        stroke: brush.stroke.clone(),
        fill,
        stroke_width: brush.stroke_width.max(1.0),
        stroke_style: brush.stroke_style,
        rotation: 0.0,
        z: next_z(scene),
        locked: false,
        group_id: None,
        text: match kind {
            BoardElementKind::Text => Some(String::new()),
            BoardElementKind::Sticky => Some("Sticky note".into()),
            _ => None,
        },
        asset_id: None,
        filename: None,
        updated_by: user_id,
    })
}

fn apply_local(scene: &mut BoardScene, op: BoardOp) -> Result<(), ()> {
    match op {
        BoardOp::Upsert { elements, .. } => {
            for element in elements {
                if let Some(existing) = scene.elements.iter_mut().find(|item| item.id == element.id) {
                    *existing = element;
                } else {
                    scene.elements.push(element);
                }
            }
        }
        BoardOp::Delete { ids, .. } => {
            scene.elements.retain(|element| !ids.contains(&element.id));
        }
        BoardOp::Reorder { ids, .. } => {
            let mut ordered = Vec::new();
            for (z, id) in ids.iter().enumerate() {
                if let Some(mut element) = scene.elements.iter().find(|item| &item.id == id).cloned() {
                    element.z = z as i64;
                    ordered.push(element);
                }
            }
            for element in &scene.elements {
                if !ids.contains(&element.id) {
                    ordered.push(element.clone());
                }
            }
            scene.elements = ordered;
        }
        BoardOp::Group { ids, group_id, .. } => {
            for element in &mut scene.elements {
                if ids.contains(&element.id) {
                    element.group_id = Some(group_id.clone());
                }
            }
        }
        BoardOp::Ungroup { group_ids, .. } => {
            for element in &mut scene.elements {
                if element.group_id.as_ref().is_some_and(|id| group_ids.contains(id)) {
                    element.group_id = None;
                }
            }
        }
    }
    scene.elements.sort_by_key(|element| element.z);
    Ok(())
}

fn next_z(scene: &BoardScene) -> i64 {
    scene.elements.iter().map(|element| element.z).max().unwrap_or(-1) + 1
}

fn new_id(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("{prefix}{nanos}")
}

fn snap(value: f64) -> f64 {
    (value / BOARD_GRID).round() * BOARD_GRID
}

fn parse_color(value: &str) -> Option<[f32; 4]> {
    let hex = value.trim().trim_start_matches('#');
    let (r, g, b, a) = match hex.len() {
        6 => (
            u8::from_str_radix(&hex[0..2], 16).ok()?,
            u8::from_str_radix(&hex[2..4], 16).ok()?,
            u8::from_str_radix(&hex[4..6], 16).ok()?,
            255u8,
        ),
        8 => (
            u8::from_str_radix(&hex[0..2], 16).ok()?,
            u8::from_str_radix(&hex[2..4], 16).ok()?,
            u8::from_str_radix(&hex[4..6], 16).ok()?,
            u8::from_str_radix(&hex[6..8], 16).ok()?,
        ),
        _ => return None,
    };
    Some([r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a as f32 / 255.0])
}

fn parse_color_u8(value: &str) -> Option<[u8; 4]> {
    let color = parse_color(value)?;
    Some([
        (color[0] * 255.0) as u8,
        (color[1] * 255.0) as u8,
        (color[2] * 255.0) as u8,
        (color[3] * 255.0) as u8,
    ])
}

const STROKE_SWATCHES: &[&str] = &[
    "#FFFFFF", "#DBDEE1", "#F23F43", "#F0B232", "#FEE75C", "#23A55A", "#00A8FC", "#5865F2", "#1E1F22",
];
const FILL_SWATCHES: &[&str] = &[
    "#F23F4388", "#F0B23288", "#FEE75C88", "#23A55A88", "#00A8FC88", "#5865F288", "#DBDEE188",
];

fn stroke_px(width: f64) -> f32 {
    (width as f32).clamp(1.0, 24.0)
}

fn finite_pt(pt: [f32; 2]) -> bool {
    pt[0].is_finite() && pt[1].is_finite()
}

fn swatch_button(ui: &Ui, id: &str, hex: &str, current: &mut String, x: f32, y: f32, none: bool) {
    let min = [x, y];
    let max = [x + BOARD_SWATCH, y + BOARD_SWATCH];
    ui.set_cursor_screen_pos(min);
    if ui.invisible_button(id, [BOARD_SWATCH, BOARD_SWATCH]) {
        *current = hex.to_string();
    }
    let chosen = current.eq_ignore_ascii_case(hex);
    let color = parse_color(hex).unwrap_or([0.0, 0.0, 0.0, 0.0]);
    if none || color[3] < 0.05 {
        skin::rect(ui, min, max, INPUT_BG, 3.0);
        ui.get_window_draw_list()
            .add_line([min[0] + 3.0, max[1] - 3.0], [max[0] - 3.0, min[1] + 3.0], MUTED)
            .thickness(1.5)
            .build();
    } else {
        skin::rect(ui, min, max, color, 3.0);
    }
    if chosen {
        ui.get_window_draw_list()
            .add_rect(min, max, WHITE)
            .thickness(2.0)
            .rounding(3.0)
            .build();
    }
}

fn paint_poly(
    ui: &Ui,
    pts: &[[f32; 2]],
    fill: [f32; 4],
    stroke: [f32; 4],
    width: f32,
    style: BoardStrokeStyle,
) {
    if pts.len() < 3 || pts.iter().any(|p| !finite_pt(*p)) {
        return;
    }
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;
    for p in pts {
        min_x = min_x.min(p[0]);
        min_y = min_y.min(p[1]);
        max_x = max_x.max(p[0]);
        max_y = max_y.max(p[1]);
    }
    // Degenerate polys trip Dear ImGui draw asserts — skip until the drag has size.
    if (max_x - min_x) < 2.0 || (max_y - min_y) < 2.0 {
        return;
    }
    if fill[3] > 0.05 {
        // Convex fill via polyline API — safer than fan triangles for diamonds.
        ui.get_window_draw_list()
            .add_polyline(pts.to_vec(), fill)
            .filled(true)
            .build();
    }
    for i in 0..pts.len() {
        let a = pts[i];
        let b = pts[(i + 1) % pts.len()];
        draw_styled_line(ui, a, b, stroke, width, style);
    }
}

fn draw_styled_line(ui: &Ui, a: [f32; 2], b: [f32; 2], color: [f32; 4], width: f32, style: BoardStrokeStyle) {
    if !finite_pt(a) || !finite_pt(b) || width <= 0.0 {
        return;
    }
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    let len = (dx * dx + dy * dy).sqrt();
    if len < 0.5 {
        return;
    }
    let draw = ui.get_window_draw_list();
    match style {
        BoardStrokeStyle::Solid => {
            draw.add_line(a, b, color).thickness(width).build();
        }
        BoardStrokeStyle::Dashed => {
            let dash = (8.0 + width * 2.0).min(len);
            let gap = dash * 0.75;
            let mut t = 0.0;
            while t < len {
                let t1 = (t + dash).min(len);
                let p0 = [a[0] + dx * (t / len), a[1] + dy * (t / len)];
                let p1 = [a[0] + dx * (t1 / len), a[1] + dy * (t1 / len)];
                draw.add_line(p0, p1, color).thickness(width).build();
                t += dash + gap;
            }
        }
        BoardStrokeStyle::Dotted => {
            let step = (4.0 + width).min(len).max(2.0);
            let mut t = 0.0;
            while t <= len {
                let p = [a[0] + dx * (t / len), a[1] + dy * (t / len)];
                draw.add_circle(p, (width * 0.55).max(1.0), color).filled(true).build();
                t += step;
            }
        }
    }
}

fn draw_rect_stroke(ui: &Ui, min: [f32; 2], max: [f32; 2], color: [f32; 4], width: f32, style: BoardStrokeStyle) {
    let corners = [
        [min[0], min[1]],
        [max[0], min[1]],
        [max[0], max[1]],
        [min[0], max[1]],
    ];
    for i in 0..4 {
        draw_styled_line(ui, corners[i], corners[(i + 1) % 4], color, width, style);
    }
}

fn fill_rect(image: &mut RgbaImage, x0: u32, y0: u32, x1: u32, y1: u32, color: [u8; 4]) {
    if color[3] == 0 {
        return;
    }
    for y in y0..=y1.min(image.height().saturating_sub(1)) {
        for x in x0..=x1.min(image.width().saturating_sub(1)) {
            image.put_pixel(x, y, Rgba(color));
        }
    }
}

fn stroke_rect(image: &mut RgbaImage, x0: u32, y0: u32, x1: u32, y1: u32, color: [u8; 4]) {
    for x in x0..=x1.min(image.width().saturating_sub(1)) {
        image.put_pixel(x, y0.min(image.height() - 1), Rgba(color));
        image.put_pixel(x, y1.min(image.height() - 1), Rgba(color));
    }
    for y in y0..=y1.min(image.height().saturating_sub(1)) {
        image.put_pixel(x0.min(image.width() - 1), y, Rgba(color));
        image.put_pixel(x1.min(image.width() - 1), y, Rgba(color));
    }
}

fn fill_ellipse(image: &mut RgbaImage, x0: u32, y0: u32, x1: u32, y1: u32, color: [u8; 4]) {
    if color[3] == 0 {
        return;
    }
    let cx = (x0 + x1) as f32 * 0.5;
    let cy = (y0 + y1) as f32 * 0.5;
    let rx = ((x1 - x0) as f32 * 0.5).max(1.0);
    let ry = ((y1 - y0) as f32 * 0.5).max(1.0);
    for y in y0..=y1.min(image.height().saturating_sub(1)) {
        for x in x0..=x1.min(image.width().saturating_sub(1)) {
            let nx = (x as f32 - cx) / rx;
            let ny = (y as f32 - cy) / ry;
            if nx * nx + ny * ny <= 1.0 {
                image.put_pixel(x, y, Rgba(color));
            }
        }
    }
}

fn stroke_ellipse(image: &mut RgbaImage, x0: u32, y0: u32, x1: u32, y1: u32, color: [u8; 4]) {
    let cx = (x0 + x1) as f32 * 0.5;
    let cy = (y0 + y1) as f32 * 0.5;
    let rx = ((x1 - x0) as f32 * 0.5).max(1.0);
    let ry = ((y1 - y0) as f32 * 0.5).max(1.0);
    for deg in 0..360 {
        let rad = (deg as f32).to_radians();
        let x = (cx + rx * rad.cos()).round() as i32;
        let y = (cy + ry * rad.sin()).round() as i32;
        if x >= 0 && y >= 0 && (x as u32) < image.width() && (y as u32) < image.height() {
            image.put_pixel(x as u32, y as u32, Rgba(color));
        }
    }
}

fn draw_line(image: &mut RgbaImage, mut x0: i32, mut y0: i32, x1: i32, y1: i32, color: [u8; 4]) {
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    loop {
        if x0 >= 0 && y0 >= 0 && (x0 as u32) < image.width() && (y0 as u32) < image.height() {
            image.put_pixel(x0 as u32, y0 as u32, Rgba(color));
        }
        if x0 == x1 && y0 == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
}
