//! The editor's audio controls: which track is heard and edited, removal, volume and EQ.

use eframe::egui::{self, Align, Layout, RichText, vec2};
use media::{Decibels, TrackEdit, TrackKeep};
use playback::AudioSelection;

use super::{Editor, EditorAction};
use crate::theme;
use crate::widgets::{self, balance_slider, pill_button, section_title};

pub const SLIDER_ROW_HEIGHT: f32 = 44.0;

#[derive(Clone, Copy)]
enum Band {
    Volume,
    Bass,
    Mid,
    Treble,
}

impl Band {
    const ALL: [Band; 4] = [Band::Volume, Band::Bass, Band::Mid, Band::Treble];

    fn label(self) -> &'static str {
        match self {
            Band::Volume => "Volume",
            Band::Bass => "Bass",
            Band::Mid => "Mid",
            Band::Treble => "Treble",
        }
    }

    /// The slider's range either side of zero, in dB.
    fn limit(self) -> f32 {
        match self {
            Band::Volume => 20.0,
            Band::Bass | Band::Mid | Band::Treble => 12.0,
        }
    }

    fn of(self, edit: &mut TrackEdit) -> &mut Decibels {
        match self {
            Band::Volume => &mut edit.gain,
            Band::Bass => &mut edit.equalizer.bass,
            Band::Mid => &mut edit.equalizer.mid,
            Band::Treble => &mut edit.equalizer.treble,
        }
    }
}

pub fn show(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    slider_rows: usize,
    actions: &mut Vec<EditorAction>,
) {
    let several = editor.audio_streams().len() > 1;
    ui.horizontal(|ui| {
        section_title(ui, "Audio");
        if several {
            track_picker(ui, editor, actions);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let (note, color) = note(editor);
            ui.add(egui::Label::new(RichText::new(note).size(12.0).color(color)).truncate());
        });
    });
    let shown = editor.shown_edit();
    let mut removed = shown.is_removed();
    let label = match editor.audio_view() {
        AudioSelection::Track(_) if several => "Remove this track",
        _ => "Remove audio",
    };
    if widgets::toggle(ui, &mut removed, label)
        .on_hover_text("Leave it out of the export; the preview plays without it")
        .changed()
    {
        let keep = if removed {
            TrackKeep::Remove
        } else {
            TrackKeep::Keep
        };
        editor.edit_audio(|edit| edit.keep = keep);
        actions.push(EditorAction::AudioChanged);
    }
    ui.add_enabled_ui(!removed, |ui| {
        if removed {
            ui.set_opacity(0.35);
        }
        sliders(ui, editor, shown, slider_rows, actions);
    });
}

/// Chooses between hearing and editing every track together, or one track on its own.
fn track_picker(ui: &mut egui::Ui, editor: &mut Editor, actions: &mut Vec<EditorAction>) {
    let view = editor.audio_view();
    let current = match view {
        AudioSelection::All => "All tracks".to_owned(),
        AudioSelection::Track(stream) => editor
            .audio_tracks()
            .find(|(s, _, _)| *s == stream)
            .map_or_else(|| "Track".to_owned(), |(_, short, _)| short.to_owned()),
    };
    // Room on the right for a drawn arrow; the UI font has no arrow glyph.
    let response = pill_button(
        ui,
        &format!("{current}     "),
        "Which track to hear and edit",
    );
    let tip = response.rect.right_center() + vec2(-12.0, 2.5);
    ui.painter().add(egui::Shape::convex_polygon(
        vec![tip, tip + vec2(-4.0, -5.0), tip + vec2(4.0, -5.0)],
        theme::TEXT_DIM,
        egui::Stroke::NONE,
    ));
    let mut picked = None;
    egui::Popup::menu(&response).show(|ui| {
        ui.set_min_width(220.0);
        if ui
            .selectable_label(view == AudioSelection::All, "All tracks")
            .on_hover_text("Hear them mixed; edits apply to every track")
            .clicked()
        {
            picked = Some(AudioSelection::All);
        }
        for (stream, _, full) in editor.audio_tracks() {
            let track = AudioSelection::Track(stream);
            if ui.selectable_label(view == track, full).clicked() {
                picked = Some(track);
            }
        }
    });
    if let Some(picked) = picked.filter(|picked| *picked != view) {
        editor.set_audio_view(picked);
        actions.push(EditorAction::AudioChanged);
    }
}

/// What the export does with the audio in view.
fn note(editor: &Editor) -> (&'static str, egui::Color32) {
    let edit = editor.shown_edit();
    if editor.viewed_edits_differ() {
        ("Tracks set separately", theme::TEXT_DIM)
    } else if edit.is_removed() {
        ("Removed from the clip", theme::keyframe())
    } else if edit.reencodes() {
        ("Re-encoded for volume and EQ", theme::smart())
    } else {
        ("Copied untouched", theme::TEXT_DIM)
    }
}

fn sliders(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    shown: TrackEdit,
    slider_rows: usize,
    actions: &mut Vec<EditorAction>,
) {
    let per_row = 4 / slider_rows.max(1);
    let gap = 14.0;
    let width =
        ((ui.available_width() - (per_row - 1) as f32 * gap) / per_row as f32).clamp(60.0, 170.0);
    let mut changed = None;
    for row in Band::ALL.chunks(per_row) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for &band in row {
                ui.allocate_ui(vec2(width, SLIDER_ROW_HEIGHT), |ui| {
                    let current = *band.of(&mut shown.clone());
                    let value = f32::from(current.0) / 10.0;
                    if let Some(new) = balance_slider(ui, band.label(), value, band.limit(), |v| {
                        format!("{v:+.1} dB")
                    }) {
                        let tenths = ((new * 2.0).round() * 5.0) as i16;
                        if tenths != current.0 {
                            changed = Some((band, Decibels(tenths)));
                        }
                    }
                });
            }
        });
    }
    if let Some((band, value)) = changed {
        editor.edit_audio(|edit| *band.of(edit) = value);
        actions.push(EditorAction::AudioChanged);
    }
}
