//! Operating system integration: file dialogs and media keys.

use std::path::PathBuf;

use crossbeam::channel::Receiver;

use crate::actions::Action;

// Drawn for the Windows taskbar; tested everywhere.
#[cfg(windows)]
mod app_icon;
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

/// The app's top-level window, as the operating system knows it (a Windows `HWND`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeWindow(#[cfg_attr(not(windows), allow(dead_code))] isize);

impl NativeWindow {
    /// The window eframe created, on Windows.
    pub fn of(creation: &eframe::CreationContext<'_>) -> Option<NativeWindow> {
        use eframe::wgpu::rwh::{HasWindowHandle, RawWindowHandle};
        match creation.window_handle().ok()?.as_raw() {
            RawWindowHandle::Win32(handle) => Some(NativeWindow(handle.hwnd.get())),
            _ => None,
        }
    }
}

/// Shows the app icon in `theme`'s colors on the window and, on Windows, wherever the shell
/// shows VibePlayer's icon (its shortcuts and the videos it opens).
pub fn show_app_icon(window: NativeWindow, theme: library::ColorTheme) {
    #[cfg(windows)]
    app_icon::show(window.0, theme);
    #[cfg(not(windows))]
    let _ = (window, theme);
}

/// Moves and sizes `window` to `placement` (pixels, outer rectangle) if that rectangle is on a
/// monitor. Returns whether it did.
pub fn place_window(window: NativeWindow, placement: library::WindowPlacement) -> bool {
    #[cfg(windows)]
    {
        windows::place_window(window.0, placement)
    }
    #[cfg(not(windows))]
    {
        let _ = (window, placement);
        false
    }
}

/// `window`'s normal rectangle in pixels and whether it's maximized; while maximized, the
/// rectangle stays `previous`'s.
pub fn window_placement(
    window: NativeWindow,
    previous: Option<library::WindowPlacement>,
) -> Option<library::WindowPlacement> {
    #[cfg(windows)]
    {
        windows::window_placement(window.0, previous)
    }
    #[cfg(not(windows))]
    {
        let _ = (window, previous);
        None
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
