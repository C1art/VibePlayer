use std::path::PathBuf;

use cut::{ClockPrecision, ClockTime, Ticks, Timebase};
use eframe::egui::{self, Align, Layout, Rect, RichText, pos2, vec2};
use media::Decibels;
use playback::SeekMode;

use super::Editor;
use super::timeline::Timeline;
use crate::icons::Icon;
use crate::theme;
use crate::widgets::{self, Kind, badge, balance_slider, button, chip, icon_button, section_title};

pub const HEIGHT: f32 = 300.0;

#[derive(Clone, Debug, PartialEq)]
pub enum EditorAction {
    Close,
    TogglePlay,
    Seek(Ticks, SeekMode),
    StepBack,
    StepForward,
    SetStartHere,
    SetEndHere,
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
    let rect = Rect::from_min_max(pos2(area.left(), area.bottom() - HEIGHT), area.max);
    ui.painter().rect_filled(rect, 0.0, theme::PANEL);
    ui.painter().line_segment(
        [rect.left_top(), rect.right_top()],
        egui::Stroke::new(1.0, theme::BORDER),
    );
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(22.0, 14.0))));
    let ui = &mut child;
    ui.spacing_mut().item_spacing = vec2(8.0, 10.0);
    let mut actions = Vec::new();

    let plan = editor.plan();
    let keyframe_start = match &plan {
        Some(Ok(plan)) if plan.start_shift() > Ticks::ZERO => {
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

    // Header.
    ui.horizontal(|ui| {
        ui.label(RichText::new("Trim").size(17.0).strong());
        if reencoded_until.is_some() {
            badge(ui, "Smart cut", theme::SMART);
        } else {
            badge(ui, "Lossless", theme::SUCCESS);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if icon_button(ui, Icon::Close, 30.0, "Back to the player (E)").clicked() {
                actions.push(EditorAction::Close);
            }
            if !editor.keyframes_complete() {
                ui.label(
                    RichText::new("Finding keyframes…")
                        .size(12.0)
                        .color(theme::TEXT_DIM),
                );
            }
        });
    });

    // Timeline.
    let keyframes: Vec<Ticks> = editor.keyframe_positions().collect();
    let input = Timeline {
        duration: editor.duration(),
        position,
        start: editor.start,
        end: editor.end,
        keyframe_start,
        reencoded_until,
        keyframes: &keyframes,
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

    // Transport and trim points.
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
        chip(ui, &clock(position));
        ui.add_space(18.0);
        ui.spacing_mut().item_spacing.x = 6.0;
        trim_point(ui, "Start", &clock(editor.start));
        if icon_button(ui, Icon::SkipBack, 28.0, "Start at the previous keyframe").clicked() {
            actions.push(EditorAction::StartToKeyframe { forward: false });
        }
        if icon_button(ui, Icon::SkipForward, 28.0, "Start at the next keyframe").clicked() {
            actions.push(EditorAction::StartToKeyframe { forward: true });
        }
        if button(ui, Kind::Secondary, None, "Set start  I").clicked() {
            actions.push(EditorAction::SetStartHere);
        }
        ui.add_space(12.0);
        trim_point(ui, "End", &clock(editor.end));
        if button(ui, Kind::Secondary, None, "Set end  O").clicked() {
            actions.push(EditorAction::SetEndHere);
        }
        ui.add_space(12.0);
        trim_point(ui, "Length", &clock(length));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if button(ui, Kind::Ghost, Some(Icon::Play), "Preview")
                .on_hover_text("Play the trimmed clip")
                .clicked()
            {
                actions.push(EditorAction::PreviewSelection);
            }
        });
    });

    let status = status(editor, plan.as_ref(), reencoded_until);
    ui.horizontal(|ui| {
        let (dot, _) = ui.allocate_exact_size(vec2(8.0, 8.0), egui::Sense::hover());
        ui.painter().circle_filled(dot.center(), 4.0, status.color);
        ui.label(RichText::new(status.text).color(theme::TEXT));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            show_result(ui, editor, &mut actions);
        });
    });

    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 28.0;
        ui.vertical(|ui| {
            section_title(ui, "Cut");
            if widgets::toggle(ui, &mut editor.snap_to_keyframes, "Snap start to keyframes")
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
        });
        ui.vertical(|ui| {
            section_title(ui, "Audio");
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 18.0;
                let mut changed = false;
                for (label, gain, limit) in [
                    ("Volume", &mut editor.gain, 20.0),
                    ("Bass", &mut editor.equalizer.bass, 12.0),
                    ("Mid", &mut editor.equalizer.mid, 12.0),
                    ("Treble", &mut editor.equalizer.treble, 12.0),
                ] {
                    ui.allocate_ui(vec2(140.0, 44.0), |ui| {
                        let value = f32::from(gain.0) / 10.0;
                        if let Some(new) =
                            balance_slider(ui, label, value, limit, |v| format!("{v:+.1} dB"))
                        {
                            let tenths = ((new * 2.0).round() * 5.0) as i16;
                            if tenths != gain.0 {
                                *gain = Decibels(tenths);
                                changed = true;
                            }
                        }
                    });
                }
                if changed {
                    editor.result = None;
                    actions.push(EditorAction::AudioChanged);
                }
            });
        });
        ui.with_layout(Layout::right_to_left(Align::Max), |ui| {
            export_area(
                ui,
                editor,
                plan.as_ref().is_some_and(Result::is_ok),
                &mut actions,
            );
        });
    });
    actions
}

fn trim_point(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.label(RichText::new(label).size(12.5).color(theme::TEXT_DIM));
    chip(ui, value);
}

fn status(
    editor: &Editor,
    plan: Option<&Result<cut::CutPlan, cut::CutError>>,
    reencoded_until: Option<Ticks>,
) -> Status {
    let audio = if editor.audio_changes().is_none() {
        ""
    } else {
        " Audio is re-encoded for the volume and EQ changes."
    };
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
                        "Frame-accurate. The whole clip is re-encoded ({} frames, {span:.2} s) because it ends before the next keyframe.{audio}",
                        head.frames
                    )
                } else {
                    format!(
                        "Frame-accurate. Re-encodes {} frames ({span:.2} s) up to the keyframe at {}; the rest is copied.{audio}",
                        head.frames,
                        clock(until)
                    )
                };
                Status {
                    text,
                    color: theme::SMART,
                }
            }
            _ if plan.start_shift() > Ticks::ZERO => {
                let shift = plan.start_shift().rescale(plan.timebase, Timebase::MICROS);
                Status {
                    text: format!(
                        "Starts at the keyframe at {}, {:.3} s before your start, so nothing is re-encoded.{audio}",
                        clock(editor.plan_start_position(plan)),
                        seconds(shift)
                    ),
                    color: theme::KEYFRAME,
                }
            }
            _ => Status {
                text: format!("Starts exactly on a keyframe. Video is copied untouched.{audio}"),
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
