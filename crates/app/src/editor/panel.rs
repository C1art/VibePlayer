use std::path::PathBuf;

use cut::{ClockPrecision, ClockTime, Ticks, Timebase};
use eframe::egui::{self, Align, Layout, Rect, RichText, pos2, vec2};
use media::Decibels;
use playback::SeekMode;

use super::Editor;
use super::timeline::Timeline;
use crate::icons::Icon;
use crate::theme;
use crate::widgets::icon_button;

pub const HEIGHT: f32 = 268.0;

#[derive(Clone, Debug, PartialEq)]
pub enum EditorAction {
    Close,
    TogglePlay,
    Seek(Ticks, SeekMode),
    StepBack,
    StepForward,
    SetStartHere,
    SetEndHere,
    PreviewSelection,
    AudioChanged,
    Export,
    Cancel,
    Open(PathBuf),
    ShowInFolder(PathBuf),
}

fn clock(ticks: Ticks) -> String {
    ClockTime::new(ticks, Timebase::MICROS, ClockPrecision::Millis).to_string()
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
        egui::Stroke::new(1.0, egui::Color32::from_rgb(44, 44, 52)),
    );
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(20.0, 14.0))));
    let ui = &mut child;
    let mut actions = Vec::new();
    let plan = editor.plan();
    let keyframe_start = match &plan {
        Some(Ok(plan)) if plan.start_shift() > Ticks::ZERO => {
            Some(editor.plan_start_position(plan))
        }
        _ => None,
    };

    ui.horizontal(|ui| {
        ui.label(RichText::new("Trim").size(17.0).strong());
        ui.label(
            RichText::new("Lossless: video is copied, never re-encoded")
                .color(theme::TEXT_DIM)
                .small(),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if icon_button(ui, Icon::Close, 30.0, "Back to player (E)").clicked() {
                actions.push(EditorAction::Close);
            }
        });
    });

    let input = Timeline {
        duration: editor.duration(),
        position,
        start: editor.start,
        end: editor.end,
        keyframe_start,
    }
    .show(ui);
    if let Some(start) = input.start {
        editor.set_start(start);
        actions.push(EditorAction::Seek(start, SeekMode::Keyframe));
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
        ui.label(RichText::new(clock(position)).monospace());
        ui.add_space(12.0);
        if ui.button("Set start  [I]").clicked() {
            actions.push(EditorAction::SetStartHere);
        }
        if ui.button("Set end  [O]").clicked() {
            actions.push(EditorAction::SetEndHere);
        }
        if ui
            .button("Preview")
            .on_hover_text("Play from the cut's start")
            .clicked()
        {
            actions.push(EditorAction::PreviewSelection);
        }
        ui.add_space(12.0);
        let length = match &plan {
            Some(Ok(plan)) => plan.duration().rescale(plan.timebase, Timebase::MICROS),
            _ => editor.end - editor.start,
        };
        ui.label(
            RichText::new(format!(
                "{}  →  {}   ({})",
                clock(editor.start),
                clock(editor.end),
                clock(length)
            ))
            .monospace(),
        );
    });

    match &plan {
        Some(Ok(plan)) if plan.start_shift() > Ticks::ZERO => {
            let shift = plan.start_shift().rescale(plan.timebase, Timebase::MICROS);
            ui.label(
                RichText::new(format!(
                    "Starts at the keyframe at {} — {:.3} s before your start, so it can be copied without re-encoding.",
                    clock(editor.plan_start_position(plan)),
                    shift.display_seconds(Timebase::MICROS)
                ))
                .color(theme::KEYFRAME),
            );
        }
        Some(Ok(_)) => {
            ui.label(RichText::new("Starts exactly on a keyframe.").color(theme::TEXT_DIM));
        }
        Some(Err(error)) => {
            ui.label(RichText::new(error.to_string()).color(theme::DANGER));
        }
        None => {
            ui.label(RichText::new("Finding the nearest keyframe…").color(theme::TEXT_DIM));
        }
    }

    ui.horizontal(|ui| {
        let mut changed = false;
        changed |= db_slider(ui, "Volume", &mut editor.gain, 200);
        ui.add_space(10.0);
        changed |= db_slider(ui, "Bass", &mut editor.equalizer.bass, 120);
        changed |= db_slider(ui, "Mid", &mut editor.equalizer.mid, 120);
        changed |= db_slider(ui, "Treble", &mut editor.equalizer.treble, 120);
        if !editor.audio_changes().is_none() && ui.small_button("Reset audio").clicked() {
            editor.gain = Decibels(0);
            editor.equalizer = media::Equalizer::default();
            changed = true;
        }
        if changed {
            editor.result = None;
            actions.push(EditorAction::AudioChanged);
        }
    });

    ui.horizontal(|ui| {
        let note = if editor.audio_changes().is_none() {
            "Nothing will be re-encoded."
        } else {
            "Video is copied; audio is re-encoded to apply volume and EQ."
        };
        ui.label(RichText::new(note).color(theme::TEXT_DIM));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if editor.is_exporting() {
                if ui.button("Cancel").clicked() {
                    actions.push(EditorAction::Cancel);
                }
                ui.add(
                    egui::ProgressBar::new(editor.export_fraction())
                        .desired_width(220.0)
                        .show_percentage(),
                );
                ui.ctx().request_repaint();
            } else {
                let ready = matches!(plan, Some(Ok(_)));
                let export = egui::Button::new(RichText::new("Export").strong())
                    .fill(theme::ACCENT)
                    .min_size(vec2(110.0, 32.0));
                if ui.add_enabled(ready, export).clicked() {
                    actions.push(EditorAction::Export);
                }
                show_result(ui, editor, &mut actions);
            }
        });
    });
    actions
}

fn show_result(ui: &mut egui::Ui, editor: &Editor, actions: &mut Vec<EditorAction>) {
    match &editor.result {
        Some(Ok(result)) => {
            if ui.button("Show in folder").clicked() {
                actions.push(EditorAction::ShowInFolder(result.output.clone()));
            }
            if ui.button("Open").clicked() {
                actions.push(EditorAction::Open(result.output.clone()));
            }
            let name = result
                .output
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            ui.label(RichText::new(format!("Saved {name}")).color(theme::ACCENT_SOFT))
                .on_hover_text(result.output.display().to_string());
        }
        Some(Err(error)) => {
            ui.label(RichText::new(error).color(theme::DANGER));
        }
        None => {}
    }
}

/// A slider for a gain in tenths of a decibel within ±`limit` tenths. Returns whether it moved.
fn db_slider(ui: &mut egui::Ui, label: &str, gain: &mut Decibels, limit: i16) -> bool {
    ui.label(RichText::new(label).color(theme::TEXT_DIM));
    let mut db = f32::from(gain.0) / 10.0;
    let max = f32::from(limit) / 10.0;
    ui.spacing_mut().slider_width = 96.0;
    let response = ui
        .add(
            egui::Slider::new(&mut db, -max..=max)
                .step_by(0.5)
                .show_value(false),
        )
        .on_hover_text("Double-click to reset");
    ui.label(
        RichText::new(format!("{:+.1} dB", f32::from(gain.0) / 10.0))
            .monospace()
            .small(),
    );
    ui.add_space(8.0);
    let tenths = (db * 10.0).round() as i16;
    if response.changed() && tenths != gain.0 {
        gain.0 = tenths;
        return true;
    }
    if response.double_clicked() && gain.0 != 0 {
        gain.0 = 0;
        return true;
    }
    false
}
