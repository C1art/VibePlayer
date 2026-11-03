//! Operating system integration: file dialogs and media keys.

use std::path::PathBuf;

use crossbeam::channel::Receiver;

use crate::actions::Action;

// Drawn for the Windows taskbar; tested everywhere.
#[cfg_attr(not(windows), allow(dead_code))]
mod glyphs;
pub mod single_instance;
#[cfg(windows)]
mod taskbar;
#[cfg(windows)]
mod windows;

/// A native dialog running on its own thread; poll for the user's choice.
pub struct PendingDialog {
    result: Receiver<Option<PathBuf>>,
}

impl PendingDialog {
    /// `Some(choice)` once the dialog closed; the choice is `None` if cancelled.
    pub fn poll(&self) -> Option<Option<PathBuf>> {
        self.result.try_recv().ok()
    }
}

/// Opens the system "Open file" dialog without blocking the UI. `None` if unsupported.
pub fn open_file_dialog() -> Option<PendingDialog> {
    #[cfg(windows)]
    {
        windows::open_file_dialog().map(|result| PendingDialog { result })
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Opens the system "Save as" dialog, starting from `suggested`. `None` if unsupported.
pub fn save_file_dialog(suggested: &std::path::Path) -> Option<PendingDialog> {
    #[cfg(windows)]
    {
        windows::save_file_dialog(suggested).map(|result| PendingDialog { result })
    }
    #[cfg(not(windows))]
    {
        let _ = suggested;
        None
    }
}

/// System media controls (play/pause keys, the volume flyout's media panel).
pub struct MediaControls {
    #[cfg(windows)]
    inner: Option<windows::MediaControls>,
}

/// What the taskbar button should show.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TaskbarState {
    /// `Some(playing)` while a file is open, for the thumbnail buttons.
    pub playing: Option<bool>,
    /// Export or download progress, 0–1.
    pub progress: Option<f32>,
}

/// The taskbar button's thumbnail toolbar and progress bar (Windows only).
pub struct Taskbar {
    #[cfg(windows)]
    inner: taskbar::Taskbar,
}

impl Taskbar {
    pub fn new(repaint: &eframe::egui::Context) -> Taskbar {
        #[cfg(windows)]
        {
            Taskbar {
                inner: taskbar::Taskbar::new(repaint.clone()),
            }
        }
        #[cfg(not(windows))]
        {
            let _ = repaint;
            Taskbar {}
        }
    }

    /// Thumbnail button presses since the last call.
    pub fn poll(&mut self) -> Vec<Action> {
        #[cfg(windows)]
        {
            self.inner.poll()
        }
        #[cfg(not(windows))]
        {
            Vec::new()
        }
    }

    pub fn update(&mut self, state: TaskbarState) {
        #[cfg(windows)]
        self.inner.update(state);
        #[cfg(not(windows))]
        let _ = state;
    }
}

/// What the media controls should currently display.
#[cfg_attr(not(windows), allow(dead_code))]
pub struct NowPlaying<'a> {
    pub title: &'a str,
    pub playing: bool,
}

impl MediaControls {
    pub fn new(creation: &eframe::CreationContext<'_>) -> MediaControls {
        #[cfg(windows)]
        {
            MediaControls {
                inner: windows::MediaControls::new(creation),
            }
        }
        #[cfg(not(windows))]
        {
            let _ = creation;
            MediaControls {}
        }
    }

    /// Media key presses since the last call.
    pub fn poll(&mut self) -> Vec<Action> {
        #[cfg(windows)]
        if let Some(inner) = &mut self.inner {
            return inner.poll();
        }
        Vec::new()
    }

    pub fn update(&mut self, now_playing: Option<NowPlaying<'_>>) {
        #[cfg(windows)]
        if let Some(inner) = &mut self.inner {
            inner.update(now_playing);
        }
        #[cfg(not(windows))]
        let _ = now_playing;
    }
}

/// Whether a window whose top-left corner is at `x`, `y` would have its title bar on a
/// screen. Positions are logical points; screens are compared in pixels, which is at least as
/// generous.
pub fn is_on_screen(x: i32, y: i32) -> bool {
    #[cfg(windows)]
    {
        windows::is_on_screen(x, y)
    }
    #[cfg(not(windows))]
    {
        let _ = (x, y);
        true
    }
}

/// Opens the file manager with `path` selected. Never blocks.
pub fn show_in_folder(path: &std::path::Path) {
    #[cfg(windows)]
    {
        let mut argument = std::ffi::OsString::from("/select,");
        argument.push(path);
        let _ = std::process::Command::new("explorer.exe")
            .arg(argument)
            .spawn();
    }
    #[cfg(not(windows))]
    if let Some(folder) = path.parent() {
        let _ = std::process::Command::new("xdg-open").arg(folder).spawn();
    }
}
