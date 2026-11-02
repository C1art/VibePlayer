use std::time::Instant;

use cut::Ticks;
use playback::SeekMode;

use crate::app::{VibePlayerApp, unix_now};
use crate::editor::{Editor, EditorAction};
use crate::os;

impl VibePlayerApp {
    pub(crate) fn toggle_editor(&mut self) {
        if self.editor.is_some() {
            self.close_editor();
            return;
        }
        let Some(session) = &mut self.session else {
            return;
        };
        let Some(opened) = session.player.opened() else {
            return;
        };
        let saved = self.history.cut_session(&opened.info.path);
        match Editor::new(&opened.info, saved) {
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
        if editor.start > Ticks::ZERO
            || editor.end < editor.duration()
            || !editor.audio_changes().is_none()
        {
            self.history.save_cut_session(editor.session(unix_now()));
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
            EditorAction::Close => self.close_editor(),
            EditorAction::SetStartHere => self.set_trim_point(true),
            EditorAction::SetEndHere => self.set_trim_point(false),
            EditorAction::Export => {
                if let Some(editor) = &mut self.editor {
                    editor.begin_export();
                }
            }
            EditorAction::Cancel => {
                if let Some(editor) = &mut self.editor {
                    editor.cancel_export();
                }
            }
            EditorAction::Open(path) => self.open(path, now),
            EditorAction::ShowInFolder(path) => os::show_in_folder(&path),
            other => {
                let (Some(session), Some(editor)) = (&mut self.session, &self.editor) else {
                    return;
                };
                let player = &mut session.player;
                match other {
                    EditorAction::TogglePlay => player.toggle_play(now),
                    EditorAction::Seek(position, mode) => player.seek(position, mode),
                    EditorAction::StepBack => player.step_backward(now),
                    EditorAction::StepForward => player.step_forward(now),
                    EditorAction::PreviewSelection => {
                        player.seek(editor.start, SeekMode::Exact);
                        player.play(now);
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
