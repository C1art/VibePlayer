//! Windows-only integration: system media controls and native file dialogs.

use std::path::{Path, PathBuf};
use std::thread;

use crossbeam::channel::{self, Receiver, Sender};
use windows::Foundation::TypedEventHandler;
use windows::Media::{
    MediaPlaybackStatus, MediaPlaybackType, SystemMediaTransportControls,
    SystemMediaTransportControlsButton, SystemMediaTransportControlsButtonPressedEventArgs,
};
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoTaskMemFree, CoUninitialize,
};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::System::WinRT::ISystemMediaTransportControlsInterop;
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FileOpenDialog, FileSaveDialog, IFileDialog, IFileOpenDialog, IFileSaveDialog, IShellItem,
    SHCreateItemFromParsingName, SIGDN_FILESYSPATH,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumThreadWindows, GetWindowTextLengthW, IsWindowVisible,
};
use windows::core::{BOOL, HSTRING, PCWSTR, Ref, w};

use super::NowPlaying;
use crate::actions::Action;

const VIDEO_PATTERNS: &str = "*.mp4;*.m4v;*.mkv;*.webm;*.mov;*.avi;*.wmv;*.ts;*.m2ts;*.mts;\
*.flv;*.mpg;*.mpeg;*.3gp;*.ogv;*.vob;*.mp3;*.m4a;*.flac;*.wav;*.ogg;*.opus";

/// The app's own top-level window, found among this thread's windows (winit creates it on
/// the main thread, which is the one calling this).
fn main_window() -> Option<HWND> {
    unsafe extern "system" fn visit(window: HWND, found: LPARAM) -> BOOL {
        // SAFETY: `found` is the address of the `Option<HWND>` below, alive for the whole
        // enumeration; the window handle comes from the system.
        unsafe {
            if IsWindowVisible(window).as_bool() && GetWindowTextLengthW(window) > 0 {
                *(found.0 as *mut Option<HWND>) = Some(window);
                return BOOL(0);
            }
        }
        BOOL(1)
    }
    let mut found: Option<HWND> = None;
    // SAFETY: the callback only writes through the pointer to `found`, which outlives the call.
    unsafe {
        let _ = EnumThreadWindows(
            GetCurrentThreadId(),
            Some(visit),
            LPARAM(&mut found as *mut Option<HWND> as isize),
        );
    }
    found
}

/// Runs a file dialog on its own thread so the UI keeps drawing.
fn spawn_dialog(
    run: impl FnOnce(Option<isize>) -> Option<PathBuf> + Send + 'static,
) -> Option<Receiver<Option<PathBuf>>> {
    let owner = main_window().map(|window| window.0 as isize);
    let (sender, receiver) = channel::bounded(1);
    thread::Builder::new()
        .name("file dialog".into())
        .spawn(move || {
            // SAFETY: COM is initialized for this thread only and uninitialized below.
            let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
            let choice = run(owner);
            if initialized {
                // SAFETY: balances the successful CoInitializeEx above on the same thread.
                unsafe { CoUninitialize() };
            }
            let _ = sender.send(choice);
        })
        .ok()?;
    Some(receiver)
}

pub fn open_file_dialog() -> Option<Receiver<Option<PathBuf>>> {
    spawn_dialog(|owner| {
        // SAFETY: COM is initialized on this thread; the dialog lives within this closure.
        unsafe {
            let dialog: IFileOpenDialog =
                CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
            let patterns = HSTRING::from(VIDEO_PATTERNS);
            let filters = [
                COMDLG_FILTERSPEC {
                    pszName: w!("Videos"),
                    pszSpec: PCWSTR(patterns.as_ptr()),
                },
                COMDLG_FILTERSPEC {
                    pszName: w!("All files"),
                    pszSpec: w!("*.*"),
                },
            ];
            dialog.SetFileTypes(&filters).ok()?;
            dialog.SetTitle(w!("Open video")).ok()?;
            show(&dialog.cast_dialog()?, owner)
        }
    })
}

pub fn save_file_dialog(suggested: &Path) -> Option<Receiver<Option<PathBuf>>> {
    let suggested = suggested.to_path_buf();
    spawn_dialog(move |owner| {
        let name = HSTRING::from(suggested.file_name()?.to_string_lossy().as_ref());
        let extension = suggested
            .extension()
            .map(|e| e.to_string_lossy().into_owned())
            .unwrap_or_default();
        let pattern = HSTRING::from(format!("*.{extension}"));
        let label = HSTRING::from(format!("{} video", extension.to_uppercase()));
        // SAFETY: COM is initialized on this thread; the dialog lives within this closure.
        unsafe {
            let dialog: IFileSaveDialog =
                CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER).ok()?;
            let filters = [COMDLG_FILTERSPEC {
                pszName: PCWSTR(label.as_ptr()),
                pszSpec: PCWSTR(pattern.as_ptr()),
            }];
            dialog.SetFileTypes(&filters).ok()?;
            dialog
                .SetDefaultExtension(&HSTRING::from(extension.as_str()))
                .ok()?;
            dialog.SetFileName(&name).ok()?;
            dialog.SetTitle(w!("Export trimmed video")).ok()?;
            if let Some(folder) = suggested.parent() {
                let folder = HSTRING::from(folder.as_os_str());
                if let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(&folder, None) {
                    let _ = dialog.SetFolder(&item);
                }
            }
            show(&dialog.cast_dialog()?, owner)
        }
    })
}

trait CastDialog {
    fn cast_dialog(&self) -> Option<IFileDialog>;
}

impl<T: windows::core::Interface> CastDialog for T {
    fn cast_dialog(&self) -> Option<IFileDialog> {
        self.cast().ok()
    }
}

/// Shows the dialog modally over `owner` and returns the chosen path.
///
/// # Safety
/// COM must be initialized on the calling thread.
unsafe fn show(dialog: &IFileDialog, owner: Option<isize>) -> Option<PathBuf> {
    let owner = owner.map(|handle| HWND(handle as *mut core::ffi::c_void));
    // SAFETY: the dialog is a valid COM object; the owner is our window or none. The returned
    // display name is CoTaskMemAlloc'd and freed after copying.
    unsafe {
        dialog.Show(owner).ok()?;
        let item = dialog.GetResult().ok()?;
        let name = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let path = name.to_string().ok().map(PathBuf::from);
        CoTaskMemFree(Some(name.0 as *const core::ffi::c_void));
        path
    }
}

/// The system media transport controls bound to the app window.
pub struct MediaControls {
    controls: Option<SystemMediaTransportControls>,
    presses: Receiver<Action>,
    sender: Sender<Action>,
    shown: Option<(String, bool)>,
    attempted: bool,
}

impl MediaControls {
    pub fn new(_creation: &eframe::CreationContext<'_>) -> Option<MediaControls> {
        let (sender, presses) = channel::unbounded();
        Some(MediaControls {
            controls: None,
            presses,
            sender,
            shown: None,
            attempted: false,
        })
    }

    /// Binds to the window the first time it's visible; media keys then work system-wide.
    fn attach(&mut self) {
        if self.attempted {
            return;
        }
        let Some(window) = main_window() else {
            return;
        };
        self.attempted = true;
        let sender = self.sender.clone();
        let attached = (|| -> windows::core::Result<SystemMediaTransportControls> {
            let interop = windows::core::factory::<
                SystemMediaTransportControls,
                ISystemMediaTransportControlsInterop,
            >()?;
            // SAFETY: `window` is a live top-level window owned by this thread.
            let controls: SystemMediaTransportControls = unsafe { interop.GetForWindow(window)? };
            controls.SetIsPlayEnabled(true)?;
            controls.SetIsPauseEnabled(true)?;
            controls.SetIsEnabled(true)?;
            controls.ButtonPressed(&TypedEventHandler::new(
                move |_: Ref<SystemMediaTransportControls>,
                      args: Ref<SystemMediaTransportControlsButtonPressedEventArgs>| {
                    if let Some(args) = args.as_ref() {
                        let action = match args.Button()? {
                            SystemMediaTransportControlsButton::Play => Some(Action::Play),
                            SystemMediaTransportControlsButton::Pause => Some(Action::Pause),
                            SystemMediaTransportControlsButton::Stop => Some(Action::Pause),
                            SystemMediaTransportControlsButton::FastForward => {
                                Some(Action::SeekBy { seconds: 10 })
                            }
                            SystemMediaTransportControlsButton::Rewind => {
                                Some(Action::SeekBy { seconds: -10 })
                            }
                            _ => None,
                        };
                        if let Some(action) = action {
                            let _ = sender.send(action);
                        }
                    }
                    Ok(())
                },
            ))?;
            Ok(controls)
        })();
        self.controls = attached.ok();
    }

    pub fn poll(&mut self) -> Vec<Action> {
        self.attach();
        self.presses.try_iter().collect()
    }

    pub fn update(&mut self, now_playing: Option<NowPlaying<'_>>) {
        let Some(controls) = &self.controls else {
            return;
        };
        let wanted = now_playing.map(|n| (n.title.to_owned(), n.playing));
        if wanted == self.shown {
            return;
        }
        let result = (|| -> windows::core::Result<()> {
            let updater = controls.DisplayUpdater()?;
            match &wanted {
                Some((title, playing)) => {
                    let status = if *playing {
                        MediaPlaybackStatus::Playing
                    } else {
                        MediaPlaybackStatus::Paused
                    };
                    controls.SetPlaybackStatus(status)?;
                    updater.SetType(MediaPlaybackType::Video)?;
                    updater
                        .VideoProperties()?
                        .SetTitle(&HSTRING::from(title.as_str()))?;
                }
                None => {
                    controls.SetPlaybackStatus(MediaPlaybackStatus::Closed)?;
                    updater.ClearAll()?;
                }
            }
            updater.Update()
        })();
        if result.is_ok() {
            self.shown = wanted;
        }
    }
}
