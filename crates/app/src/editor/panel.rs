use std::path::PathBuf;

use cut::{ClockPrecision, ClockTime, Ticks, Timebase};
use eframe::egui::{self, Align, Layout, Rect, RichText, pos2, vec2};
use playback::SeekMode;

use super::Editor;
use super::timeline::Timeline;
use crate::icons::Icon;
use crate::theme;
use crate::widgets::{self, Kind, badge, button, chip, icon_button, section_title};

const PADDING: egui::Vec2 = vec2(22.0, 14.0);
const CARD_GAP: f32 = 12.0;
const CLIP_WIDTH: f32 = 290.0;
const CUT_WIDTH: f32 = 290.0;
/// Narrower than this, the audio sliders go two to a row.
const AUDIO_ONE_ROW_WIDTH: f32 = 470.0;
use super::audio_card::SLIDER_ROW_HEIGHT;

/// The panel's height in a window `width` points wide: taller when narrow, so the audio
/// sliders have room.
pub fn height(width: f32) -> f32 {
    300.0 + extra_slider_rows(width) as f32 * (SLIDER_ROW_HEIGHT + 6.0)
}

fn extra_slider_rows(width: f32) -> usize {
    let audio = width - 2.0 * PADDING.x - CLIP_WIDTH - CUT_WIDTH - 2.0 * CARD_GAP;
    usize::from(audio < AUDIO_ONE_ROW_WIDTH)
}

#[derive(Clone, Debug, PartialEq)]
pub enum EditorAction {
    Close,
    OpenFile,
    TogglePlay,
    Seek(Ticks, SeekMode),
    StepBack,
    StepForward,
    StartToKeyframe { forward: bool },
    PreviewSelection,
    AudioChanged,
    PreferencesChanged,
    Export,
    Cancel,
    Open(PathBuf),
    ShowInFolder(PathBuf),
}

fn clock(ticks: Ticks) -> String {
    ClockTime::new(ticks, Timebase::MICROS, ClockPrecision::Millis).to_string()
}

fn seconds(ticks: Ticks) -> f64 {
    ticks.display_seconds(Timebase::MICROS)
}

/// What the plan means for the user, in one line and a color.
struct Status {
    text: String,
    color: egui::Color32,
}

pub fn show(
    ui: &mut egui::Ui,
    area: Rect,
    editor: &mut Editor,
    position: Ticks,
    playing: bool,
) -> Vec<EditorAction> {
    let rect = Rect::from_min_max(
        pos2(area.left(), area.bottom() - height(area.width())),
        area.max,
    );
    let extra_rows = extra_slider_rows(area.width());
    let card_height = CARD_HEIGHT + extra_rows as f32 * (SLIDER_ROW_HEIGHT + 6.0);
    ui.painter().rect_filled(rect, 0.0, theme::PANEL);
    ui.painter().line_segment(
        [rect.left_top(), rect.right_top()],
        egui::Stroke::new(1.0, theme::BORDER),
    );
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(PADDING)));
    let ui = &mut child;
    ui.spacing_mut().item_spacing = vec2(8.0, 10.0);
    let mut actions = Vec::new();

    let plan = editor.plan();
    let keyframe_start = match &plan {
        Some(Ok(plan)) if plan.start_shift() != Ticks::ZERO => {
            Some(editor.plan_start_position(plan))
        }
        _ => None,
    };
    let reencoded_until = match &plan {
        Some(Ok(plan)) => plan
            .reencode
            .map(|head| editor.stream_to_position(head.until)),
        _ => None,
    };
    let length = match &plan {
        Some(Ok(plan)) => plan.duration().rescale(plan.timebase, Timebase::MICROS),
        _ => editor.end - editor.start,
    };

    timeline(
        ui,
        editor,
        position,
        keyframe_start,
        reencoded_until,
        &mut actions,
    );
    transport(ui, editor, position, playing, &mut actions);

    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = CARD_GAP;
        card(ui, CLIP_WIDTH, card_height, |ui| {
            clip_card(ui, editor, length, &mut actions);
        });
        card(ui, CUT_WIDTH, card_height, |ui| {
            cut_card(ui, editor, &mut actions);
        });
        let rest = ui.available_width();
        card(ui, rest, card_height, |ui| {
            super::audio_card::show(ui, editor, 1 + extra_rows, &mut actions);
        });
    });

    let status = status(editor, plan.as_ref(), reencoded_until);
    let ready = plan.as_ref().is_some_and(Result::is_ok);
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        export_area(ui, editor, ready, &mut actions);
        show_result(ui, editor, &mut actions);
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            if reencoded_until.is_some() {
                badge(ui, "Smart cut", theme::smart());
            } else {
                badge(ui, "Lossless", theme::SUCCESS);
            }
            let (dot, _) = ui.allocate_exact_size(vec2(8.0, 8.0), egui::Sense::hover());
            ui.painter().circle_filled(dot.center(), 4.0, status.color);
            ui.add(egui::Label::new(RichText::new(&status.text).color(theme::TEXT)).truncate());
        });
    });
    actions
}

const CARD_HEIGHT: f32 = 104.0;

fn timeline(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    position: Ticks,
    keyframe_start: Option<Ticks>,
    reencoded_until: Option<Ticks>,
    actions: &mut Vec<EditorAction>,
) {
    let keyframes: Vec<Ticks> = editor.keyframe_positions().collect();
    let input = Timeline {
        duration: editor.duration(),
        position,
        start: editor.start,
        end: editor.end,
        keyframe_start,
        reencoded_until,
        keyframes: &keyframes,
        waveform: editor.waveform(),
    }
    .show(ui);
    if let Some(start) = input.start {
        editor.set_start(start);
        actions.push(EditorAction::Seek(editor.start, SeekMode::Keyframe));
    }
    if let Some(end) = input.end {
        editor.set_end(end);
        actions.push(EditorAction::Seek(end, SeekMode::Keyframe));
    }
    if let Some(at) = input.scrub {
        actions.push(EditorAction::Seek(at, SeekMode::Keyframe));
    }
    if let Some(at) = input.seek {
        actions.push(EditorAction::Seek(at, SeekMode::Exact));
    }
}

/// Playback on the left; previewing, opening and leaving on the right.
fn transport(
    ui: &mut egui::Ui,
    editor: &Editor,
    position: Ticks,
    playing: bool,
    actions: &mut Vec<EditorAction>,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let play = if playing { Icon::Pause } else { Icon::Play };
        if icon_button(ui, play, 34.0, "Play / pause (Space)").clicked() {
            actions.push(EditorAction::TogglePlay);
        }
        if icon_button(ui, Icon::StepBack, 30.0, "Previous frame (,)").clicked() {
            actions.push(EditorAction::StepBack);
        }
        if icon_button(ui, Icon::StepForward, 30.0, "Next frame (.)").clicked() {
            actions.push(EditorAction::StepForward);
        }
        ui.add_space(6.0);
        chip(ui, &clock(position));
        ui.label(
            RichText::new(format!("/ {}", clock(editor.duration())))
                .size(12.5)
                .color(theme::TEXT_DIM),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if icon_button(ui, Icon::Close, 30.0, "Back to the player (E)").clicked() {
                actions.push(EditorAction::Close);
            }
            if icon_button(ui, Icon::Folder, 30.0, "Open another file (Ctrl + O)").clicked() {
                actions.push(EditorAction::OpenFile);
            }
            ui.add_space(8.0);
            if button(ui, Kind::Secondary, Some(Icon::Play), "Preview clip")
                .on_hover_text("Play from the start point to the end point")
                .clicked()
            {
                actions.push(EditorAction::PreviewSelection);
            }
            let busy = match (editor.keyframes_complete(), editor.waveform_complete()) {
                (false, _) => Some("Finding keyframes…"),
                (true, false) => Some("Reading audio…"),
                (true, true) => None,
            };
            if let Some(busy) = busy {
                ui.add_space(8.0);
                ui.label(RichText::new(busy).size(12.0).color(theme::TEXT_DIM));
            }
        });
    });
}

/// A bordered group of controls, all the same height.
fn card(ui: &mut egui::Ui, width: f32, height: f32, contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, theme::BORDER))
        .corner_radius(10)
        .inner_margin(egui::Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width((width - 30.0).max(120.0));
            ui.set_height(height);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                contents(ui);
            });
        });
}

/// A card's heading, with an optional note on the right.
fn card_title(ui: &mut egui::Ui, title: &str, note: Option<(&str, egui::Color32)>) {
    ui.horizontal(|ui| {
        section_title(ui, title);
        if let Some((note, color)) = note {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(note).size(12.0).color(color));
            });
        }
    });
}

fn clip_card(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    length: Ticks,
    actions: &mut Vec<EditorAction>,
) {
    let length = format!("Length {}", clock(length));
    card_title(ui, "Clip", Some((&length, theme::TEXT_DIM)));
    egui::Grid::new("trim points")
        .num_columns(3)
        .spacing(vec2(6.0, 6.0))
        .show(ui, |ui| {
            trim_label(ui, "Start");
            chip(ui, &clock(editor.start));
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                if icon_button(
                    ui,
                    Icon::SkipBack,
                    28.0,
                    "Move the start to the previous keyframe",
                )
                .clicked()
                {
                    actions.push(EditorAction::StartToKeyframe { forward: false });
                }
                if icon_button(
                    ui,
                    Icon::SkipForward,
                    28.0,
                    "Move the start to the next keyframe",
                )
                .clicked()
                {
                    actions.push(EditorAction::StartToKeyframe { forward: true });
                }
            });
            ui.end_row();
            trim_label(ui, "End");
            chip(ui, &clock(editor.end));
            ui.end_row();
        });
}

fn trim_label(ui: &mut egui::Ui, label: &str) {
    ui.label(RichText::new(label).size(12.5).color(theme::TEXT_DIM));
}

fn cut_card(ui: &mut egui::Ui, editor: &mut Editor, actions: &mut Vec<EditorAction>) {
    card_title(ui, "Cut", None);
    if widgets::toggle(ui, &mut editor.snap_to_keyframes, "Snap start to keyframes")
        .on_hover_text("Dragging the start sticks to keyframes, where a lossless cut can begin")
        .changed()
    {
        actions.push(EditorAction::PreferencesChanged);
    }
    let available = editor.smart_cut_unavailable.is_none();
    let response = widgets::toggle_enabled(
        ui,
        &mut editor.smart_cut,
        "Frame-accurate start (smart cut)",
        available,
    );
    let response = match &editor.smart_cut_unavailable {
        Some(reason) => response
            .on_disabled_hover_text(reason.clone())
            .on_hover_text(reason.clone()),
        None => response.on_hover_text(
            "Re-encodes only the frames up to the next keyframe; the rest is still copied",
        ),
    };
    if response.changed() {
        editor.result = None;
        actions.push(EditorAction::PreferencesChanged);
    }
}

fn status(
    editor: &Editor,
    plan: Option<&Result<cut::CutPlan, cut::CutError>>,
    reencoded_until: Option<Ticks>,
) -> Status {
    match plan {
        None => Status {
            text: "Finding the keyframes around the start…".into(),
            color: theme::TEXT_DIM,
        },
        Some(Err(error)) => Status {
            text: format!("Can't cut here: {error}."),
            color: theme::DANGER,
        },
        Some(Ok(plan)) => match (plan.reencode, reencoded_until) {
            (Some(head), Some(until)) => {
                let span = seconds(until - editor.start);
                let text = if until >= editor.end {
                    format!(
                        "Frame-accurate. The whole clip is re-encoded ({} frames, {span:.2} s) because it ends before the next keyframe.",
                        head.frames
                    )
                } else {
                    format!(
                        "Frame-accurate. Re-encodes {} frames ({span:.2} s) up to the keyframe at {}; the rest is copied.",
                        head.frames,
                        clock(until)
                    )
                };
                Status {
                    text,
                    color: theme::smart(),
                }
            }
            _ if plan.start_shift() < Ticks::ZERO => Status {
                text: format!(
                    "Starts at the first keyframe at {}, {:.3} s after your start: nothing \
                     before it can be decoded.",
                    clock(editor.plan_start_position(plan)),
                    seconds(Ticks(-plan.start_shift().0).rescale(plan.timebase, Timebase::MICROS))
                ),
                color: theme::keyframe(),
            },
            _ if plan.start_shift() > Ticks::ZERO => {
                let shift = plan.start_shift().rescale(plan.timebase, Timebase::MICROS);
                Status {
                    text: format!(
                        "Starts at the keyframe at {}, {:.3} s before your start, so nothing is re-encoded.",
                        clock(editor.plan_start_position(plan)),
                        seconds(shift)
                    ),
                    color: theme::keyframe(),
                }
            }
            _ => Status {
                text: "Starts exactly on a keyframe. Video is copied untouched.".into(),
                color: theme::SUCCESS,
            },
        },
    }
}

fn export_area(ui: &mut egui::Ui, editor: &Editor, ready: bool, actions: &mut Vec<EditorAction>) {
    if editor.is_exporting() {
        if button(ui, Kind::Secondary, None, "Cancel").clicked() {
            actions.push(EditorAction::Cancel);
        }
        ui.add(
            egui::ProgressBar::new(editor.export_fraction())
                .desired_width(200.0)
                .desired_height(8.0)
                .corner_radius(4.0),
        );
        ui.ctx().request_repaint();
        return;
    }
    if widgets::button_enabled(ui, Kind::Primary, Some(Icon::Scissors), "Export", ready).clicked() {
        actions.push(EditorAction::Export);
    }
}

/// The outcome of the last export, laid out right to left.
fn show_result(ui: &mut egui::Ui, editor: &Editor, actions: &mut Vec<EditorAction>) {
    match &editor.result {
        Some(Ok(result)) => {
            if button(ui, Kind::Ghost, Some(Icon::Folder), "Show").clicked() {
                actions.push(EditorAction::ShowInFolder(result.output.clone()));
            }
            if button(ui, Kind::Ghost, Some(Icon::Play), "Open").clicked() {
                actions.push(EditorAction::Open(result.output.clone()));
            }
            let name = result
                .output
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            ui.label(RichText::new(format!("Saved {name}")).color(theme::SUCCESS))
                .on_hover_text(result.output.display().to_string());
        }
        Some(Err(error)) => {
            ui.label(RichText::new(error).color(theme::DANGER));
        }
        None => {}
    }
}
