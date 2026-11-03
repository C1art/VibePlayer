use std::time::Instant;

use cut::Ticks;
use eframe::egui;
use playback::SeekMode;

use crate::app::{VibePlayerApp, unix_now};
use crate::editor::{Editor, EditorAction};
use crate::os;

impl VibePlayerApp {
    /// Says so and returns `true` while an export runs, which closing the editor would cancel.
    pub(crate) fn export_in_the_way(&mut self) -> bool {
        let exporting = self.editor.as_ref().is_some_and(Editor::is_exporting);
        if exporting {
            self.osd.show("Finish or cancel the export first");
        }
        exporting
    }

    pub(crate) fn toggle_editor(&mut self, ctx: &egui::Context) {
        if self.editor.is_some() {
            if !self.export_in_the_way() {
                self.close_editor();
            }
            return;
        }
        let Some(session) = &mut self.session else {
            return;
        };
        let Some(opened) = session.player.opened() else {
            return;
        };
        let saved = self.history.cut_session(&opened.info.path);
        match Editor::new(
            &opened.info,
            opened.audio,
            saved,
            &self.settings.editor,
            ctx,
        ) {
            Some(editor) => {
                session
                    .player
                    .set_audio_edits(editor.gain, editor.equalizer);
                self.editor = Some(editor);
                self.info_open = false;
            }
            None => self.osd.show("This file can't be trimmed"),
        }
    }

    /// Leaves the editor, remembering its trim and dropping its audio preview.
    pub(crate) fn close_editor(&mut self) {
        let Some(editor) = self.editor.take() else {
            return;
        };
        let edited = editor.start > Ticks::ZERO
            || editor.end < editor.duration()
            || !editor.audio_changes().is_none();
        if edited {
            self.history.save_cut_session(editor.session(unix_now()));
            self.save_history();
        } else if self.history.forget_cut_session(editor.source()) {
            // Reset to the whole file: an older trim must not come back next time.
            self.save_history();
        }
        if let Some(session) = &mut self.session {
            session
                .player
                .set_audio_edits(media::Decibels(0), media::Equalizer::default());
        }
    }

    pub(crate) fn set_trim_point(&mut self, start: bool) {
        let (Some(editor), Some(session)) = (&mut self.editor, &self.session) else {
            return;
        };
        let position = session
            .player
            .current_frame()
            .map_or(session.player.position(), |frame| frame.pts);
        if start {
            editor.set_start(position);
            self.osd.show("Start set");
        } else {
            editor.set_end(position);
            self.osd.show("End set");
        }
    }

    pub(crate) fn apply_editor(&mut self, action: EditorAction, now: Instant) {
        match action {
            EditorAction::Close => {
                if !self.export_in_the_way() {
                    self.close_editor();
                }
            }
            EditorAction::OpenFile => {
                if self.open_dialog.is_none() {
                    self.open_dialog = os::open_file_dialog();
                    if self.open_dialog.is_none() {
                        self.osd.show("Drop a file onto the window to open it");
                    }
                }
            }
            EditorAction::SetStartHere => self.set_trim_point(true),
            EditorAction::SetEndHere => self.set_trim_point(false),
            EditorAction::Export => {
                if let Some(editor) = &mut self.editor {
                    editor.begin_export();
                }
            }
            EditorAction::PreferencesChanged => {
                if let Some(editor) = &self.editor {
                    self.settings.editor = editor.preferences();
                    self.save_settings();
                }
            }
            EditorAction::StartToKeyframe { forward } => {
                let moved = self
                    .editor
                    .as_mut()
                    .and_then(|e| e.step_start_to_keyframe(forward));
                match (moved, &mut self.session) {
                    (Some(start), Some(session)) => session.player.seek(start, SeekMode::Exact),
                    _ => self.osd.show("No keyframe that way"),
                }
            }
            EditorAction::Cancel => {
                if let Some(editor) = &mut self.editor {
                    editor.cancel_export();
                }
            }
            EditorAction::Open(path) => {
                if !self.export_in_the_way() {
                    self.open(path, now);
                }
            }
            EditorAction::ShowInFolder(path) => os::show_in_folder(&path),
            other => {
                let (Some(session), Some(editor)) = (&mut self.session, &mut self.editor) else {
                    return;
                };
                let player = &mut session.player;
                match other {
                    EditorAction::TogglePlay => player.toggle_play(now),
                    EditorAction::Seek(position, mode) => {
                        player.seek(position, mode);
                        editor.previewing = false;
                    }
                    EditorAction::StepBack => player.step_backward(now),
                    EditorAction::StepForward => player.step_forward(now),
                    EditorAction::PreviewSelection => {
                        player.seek(editor.start, SeekMode::Exact);
                        player.play(now);
                        editor.previewing = true;
                    }
                    EditorAction::AudioChanged => {
                        player.set_audio_edits(editor.gain, editor.equalizer);
                    }
                    _ => {}
                }
            }
        }
    }
}

impl VibePlayerApp {
    /// A selection preview pauses at the out point.
    pub(crate) fn end_preview(&mut self) {
        let (Some(editor), Some(session)) = (&mut self.editor, &mut self.session) else {
            return;
        };
        if !editor.previewing {
            return;
        }
        let player = &mut session.player;
        if !player.is_playing() || player.is_seeking() {
            editor.previewing = player.is_seeking();
            return;
        }
        if player.position() >= editor.end {
            player.pause();
            editor.previewing = false;
        }
    }
}
