//! The keyboard shortcuts window: grouped rows, each with its keys drawn as keycaps.

use eframe::egui::{self, Color32, FontId, Rect, Sense, Stroke, StrokeKind, pos2, vec2};

use crate::theme;
use crate::widgets;

/// One key as printed on the keyboard.
#[derive(Clone, Copy)]
enum Cap {
    Text(&'static str),
    Left,
    Right,
    Up,
    Down,
}

use Cap::{Down, Left, Right, Text, Up};

/// Keys pressed together, e.g. Ctrl + O.
type Combo = &'static [Cap];

struct Row {
    what: &'static str,
    keys: Combo,
}

const fn row(what: &'static str, keys: Combo) -> Row {
    Row { what, keys }
}

const SECTIONS: &[(&str, &[Row])] = &[
    (
        "Playback",
        &[
            row("Play or pause", &[Text("Space")]),
            row("Back", &[Left]),
            row("Forward", &[Right]),
            row("Previous frame", &[Text(",")]),
            row("Next frame", &[Text(".")]),
            row("Slower", &[Text("[")]),
            row("Faster", &[Text("]")]),
        ],
    ),
    (
        "Sound and tracks",
        &[
            row("Volume up", &[Up]),
            row("Volume down", &[Down]),
            row("Mute", &[Text("M")]),
            row("Next audio track", &[Text("A")]),
            row("Next subtitle track", &[Text("S")]),
        ],
    ),
    (
        "View",
        &[
            row("Fullscreen", &[Text("F")]),
            row("Leave fullscreen or close a window", &[Text("Esc")]),
            row("File info", &[Text("Tab")]),
        ],
    ),
    (
        "Trim editor",
        &[
            row("Open or close the editor", &[Text("E")]),
            row("Set the start at the playhead", &[Text("I")]),
            row("Set the end at the playhead", &[Text("O")]),
        ],
    ),
    (
        "Files and app",
        &[
            row("Open file", &[Text("Ctrl"), Text("O")]),
            row("Close file", &[Text("Ctrl"), Text("W")]),
            row("Settings", &[Text("Ctrl"), Text(",")]),
            row("Keyboard shortcuts", &[Text("F1")]),
        ],
    ),
];

const WIDTH: f32 = 400.0;
const ROW_HEIGHT: f32 = 34.0;
const CAP_HEIGHT: f32 = 26.0;
/// Room kept on the right for the scroll bar.
const SCROLL_BAR_ROOM: f32 = 16.0;

pub fn show(ctx: &egui::Context, open: &mut bool) {
    let max_height = (ctx.content_rect().height() * 0.75).clamp(200.0, 560.0);
    widgets::window("Keyboard shortcuts")
        .open(open)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            ui.set_width(WIDTH);
            egui::ScrollArea::vertical()
                .max_height(max_height)
                .min_scrolled_height(max_height)
                .show(ui, |ui| {
                    ui.set_width(WIDTH);
                    for (index, (title, rows)) in SECTIONS.iter().enumerate() {
                        if index > 0 {
                            ui.add_space(12.0);
                        }
                        widgets::section_title(ui, title);
                        ui.add_space(2.0);
                        for row in *rows {
                            shortcut_row(ui, row);
                        }
                    }
                });
        });
}

fn shortcut_row(ui: &mut egui::Ui, row: &Row) {
    let (rect, _) =
        ui.allocate_exact_size(vec2(WIDTH - SCROLL_BAR_ROOM, ROW_HEIGHT), Sense::hover());
    ui.painter().text(
        rect.left_center(),
        egui::Align2::LEFT_CENTER,
        row.what,
        FontId::proportional(14.0),
        theme::TEXT,
    );
    // Keys are laid out from the right edge inwards.
    let mut right = rect.right();
    for (index, cap) in row.keys.iter().rev().enumerate() {
        if index > 0 {
            let plus = pos2(right - 7.0, rect.center().y);
            ui.painter().text(
                plus,
                egui::Align2::CENTER_CENTER,
                "+",
                FontId::proportional(13.0),
                theme::TEXT_DIM,
            );
            right -= 14.0;
        }
        right = keycap(ui, *cap, right, rect.center().y);
    }
}

/// What a keycap shows.
enum Face {
    Label(std::sync::Arc<egui::Galley>),
    Arrow(egui::Vec2),
}

/// Draws a keycap ending at `right`, centered on `center_y`. Returns its left edge.
fn keycap(ui: &egui::Ui, cap: Cap, right: f32, center_y: f32) -> f32 {
    let painter = ui.painter();
    let face = match cap {
        Text(text) => Face::Label(painter.layout_no_wrap(
            text.to_owned(),
            FontId::proportional(13.0),
            Color32::PLACEHOLDER,
        )),
        Left => Face::Arrow(vec2(-1.0, 0.0)),
        Right => Face::Arrow(vec2(1.0, 0.0)),
        Up => Face::Arrow(vec2(0.0, -1.0)),
        Down => Face::Arrow(vec2(0.0, 1.0)),
    };
    let width = match &face {
        Face::Label(galley) => (galley.size().x + 18.0).max(CAP_HEIGHT),
        Face::Arrow(_) => CAP_HEIGHT,
    };
    let rect = Rect::from_min_max(
        pos2(right - width, center_y - CAP_HEIGHT / 2.0),
        pos2(right, center_y + CAP_HEIGHT / 2.0),
    );
    // A slightly deeper edge underneath makes it read as a key.
    painter.rect_filled(rect.translate(vec2(0.0, 2.0)), 6.0, theme::BORDER);
    painter.rect(
        rect,
        6.0,
        theme::PANEL_RAISED,
        Stroke::new(1.0, theme::BORDER),
        StrokeKind::Inside,
    );
    match face {
        Face::Label(galley) => {
            let at = rect.center() - galley.size() / 2.0;
            painter.galley(at, galley, theme::TEXT);
        }
        Face::Arrow(direction) => arrow(painter, rect.center(), direction),
    }
    rect.left()
}

fn arrow(painter: &egui::Painter, center: egui::Pos2, direction: egui::Vec2) {
    let side = vec2(-direction.y, direction.x);
    let reach = 6.0;
    let tip = center + direction * reach;
    let tail = center - direction * reach;
    let stroke = Stroke::new(1.6, theme::TEXT);
    painter.line_segment([tail, tip], stroke);
    let head = 4.0;
    painter.line_segment([tip, tip - direction * head + side * head], stroke);
    painter.line_segment([tip, tip - direction * head - side * head], stroke);
}
