//! The taskbar button: thumbnail toolbar buttons (back, play/pause, forward) and a progress
//! bar for exports and update downloads.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crossbeam::channel::{self, Receiver, Sender};
use eframe::egui;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{CreateBitmap, DeleteObject, HGDIOBJ};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::UI::Shell::{
    DefSubclassProc, ITaskbarList3, RemoveWindowSubclass, SetWindowSubclass, TBPF_NOPROGRESS,
    TBPF_NORMAL, THB_FLAGS, THB_ICON, THB_TOOLTIP, THBF_ENABLED, THBF_HIDDEN, THBN_CLICKED,
    THUMBBUTTON, TaskbarList,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, DestroyIcon, GetSystemMetrics, HICON, ICONINFO, RegisterWindowMessageW,
    SM_CXSMICON, WM_COMMAND,
};
use windows::core::w;

use super::TaskbarState;
use super::glyphs::{self, Glyph};
use super::windows::main_window;
use crate::actions::Action;

const BACK: u32 = 1;
const PLAY_PAUSE: u32 = 2;
const FORWARD: u32 = 3;
const SUBCLASS_ID: usize = 0x5650; // "VP"
/// How often to try adding the buttons again while the taskbar isn't ready for them.
const RETRY: Duration = Duration::from_secs(1);

/// Lives as long as the window subclass that points at it.
struct Shared {
    clicks: Sender<Action>,
    repaint: egui::Context,
    button_created: u32,
    recreated: AtomicBool,
}

pub struct Taskbar {
    shared: Box<Shared>,
    clicks: Receiver<Action>,
    attached: Option<(HWND, ITaskbarList3)>,
    attempted: bool,
    icons: Option<[HICON; 4]>,
    buttons_added: bool,
    last_try: Option<Instant>,
    shown: Option<TaskbarState>,
}

impl Taskbar {
    pub fn new(repaint: egui::Context) -> Taskbar {
        let (clicks, receiver) = channel::unbounded();
        // SAFETY: registering a message name has no preconditions.
        let button_created = unsafe { RegisterWindowMessageW(w!("TaskbarButtonCreated")) };
        Taskbar {
            shared: Box::new(Shared {
                clicks,
                repaint,
                button_created,
                recreated: AtomicBool::new(false),
            }),
            clicks: receiver,
            attached: None,
            attempted: false,
            icons: None,
            buttons_added: false,
            last_try: None,
            shown: None,
        }
    }

    /// Thumbnail button presses since the last call.
    pub fn poll(&mut self) -> Vec<Action> {
        self.clicks.try_iter().collect()
    }

    pub fn update(&mut self, state: TaskbarState) {
        self.attach();
        if self.shared.recreated.swap(false, Ordering::Relaxed) {
            // Explorer restarted: the buttons and progress must be set again.
            self.buttons_added = false;
            self.shown = None;
        }
        let Some((window, list)) = &self.attached else {
            return;
        };
        let (window, list) = (*window, list.clone());
        if !self.buttons_added && self.last_try.is_none_or(|last| last.elapsed() >= RETRY) {
            self.last_try = Some(Instant::now());
            let buttons = self.buttons(state.playing);
            // SAFETY: `window` is our live top-level window; the buttons are fully initialized.
            self.buttons_added = unsafe { list.ThumbBarAddButtons(window, &buttons) }.is_ok();
            if self.buttons_added {
                self.shown = Some(TaskbarState {
                    playing: state.playing,
                    progress: None,
                });
            }
        }
        let shown = self.shown.unwrap_or(TaskbarState {
            playing: None,
            progress: None,
        });
        if self.buttons_added && shown.playing != state.playing {
            let buttons = self.buttons(state.playing);
            // SAFETY: as above.
            let _ = unsafe { list.ThumbBarUpdateButtons(window, &buttons) };
        }
        if shown.progress != state.progress {
            // SAFETY: `window` is our live top-level window.
            let _ = unsafe {
                match state.progress {
                    Some(fraction) => list.SetProgressState(window, TBPF_NORMAL).and_then(|()| {
                        let done = (fraction.clamp(0.0, 1.0) * 1000.0) as u64;
                        list.SetProgressValue(window, done, 1000)
                    }),
                    None => list.SetProgressState(window, TBPF_NOPROGRESS),
                }
            };
        }
        self.shown = Some(state);
    }

    /// Subclasses the window to hear button clicks, once it exists.
    fn attach(&mut self) {
        if self.attempted {
            return;
        }
        let Some(window) = main_window() else {
            return;
        };
        self.attempted = true;
        // SAFETY: COM is initialized on the UI thread by the windowing library.
        let Ok(list) = (unsafe {
            CoCreateInstance::<_, ITaskbarList3>(&TaskbarList, None, CLSCTX_INPROC_SERVER)
        }) else {
            return;
        };
        // SAFETY: `list` is a fresh taskbar list object.
        if unsafe { list.HrInit() }.is_err() {
            return;
        }
        let data = std::ptr::from_ref::<Shared>(&self.shared) as usize;
        // SAFETY: the subclass is removed in `drop` before `shared` is freed, and the window
        // belongs to this thread.
        if !unsafe { SetWindowSubclass(window, Some(subclass), SUBCLASS_ID, data) }.as_bool() {
            return;
        }
        self.icons = make_icons();
        self.attached = Some((window, list));
    }

    fn buttons(&self, playing: Option<bool>) -> [THUMBBUTTON; 3] {
        let icon = |index: usize| self.icons.map(|icons| icons[index]).unwrap_or_default();
        let flags = if playing.is_some() {
            THBF_ENABLED
        } else {
            THBF_HIDDEN
        };
        let play_pause = if playing == Some(true) {
            (icon(1), "Pause")
        } else {
            (icon(0), "Play")
        };
        [
            button(BACK, icon(2), "Back 10 seconds", flags),
            button(PLAY_PAUSE, play_pause.0, play_pause.1, flags),
            button(FORWARD, icon(3), "Forward 10 seconds", flags),
        ]
    }
}

impl Drop for Taskbar {
    fn drop(&mut self) {
        if let Some((window, _)) = &self.attached {
            // SAFETY: removes the subclass set in `attach`, before `shared` goes away.
            let _ = unsafe { RemoveWindowSubclass(*window, Some(subclass), SUBCLASS_ID) };
        }
        for icon in self.icons.into_iter().flatten() {
            // SAFETY: the icons were created by `make_icons` and are no longer in use.
            let _ = unsafe { DestroyIcon(icon) };
        }
    }
}

fn button(
    id: u32,
    icon: HICON,
    tip: &str,
    flags: windows::Win32::UI::Shell::THUMBBUTTONFLAGS,
) -> THUMBBUTTON {
    let mut tooltip = [0u16; 260];
    for (slot, unit) in tooltip.iter_mut().zip(tip.encode_utf16().take(259)) {
        *slot = unit;
    }
    THUMBBUTTON {
        dwMask: THB_ICON | THB_TOOLTIP | THB_FLAGS,
        iId: id,
        iBitmap: 0,
        hIcon: icon,
        szTip: tooltip,
        dwFlags: flags,
    }
}

/// Play, pause, back and forward, at the system's small icon size.
fn make_icons() -> Option<[HICON; 4]> {
    // SAFETY: GetSystemMetrics only reads a system value.
    let size = u32::try_from(unsafe { GetSystemMetrics(SM_CXSMICON) })
        .ok()
        .filter(|&size| size > 0)
        .unwrap_or(16);
    let icons = [Glyph::Play, Glyph::Pause, Glyph::Back, Glyph::Forward]
        .map(|glyph| make_icon(&glyphs::render(glyph, size), size));
    if icons.iter().all(Option::is_some) {
        Some(icons.map(Option::unwrap_or_default))
    } else {
        for icon in icons.into_iter().flatten() {
            // SAFETY: created just above and not handed out.
            let _ = unsafe { DestroyIcon(icon) };
        }
        None
    }
}

fn make_icon(bgra: &[u8], size: u32) -> Option<HICON> {
    let side = i32::try_from(size).ok()?;
    let mask = vec![0u8; (size as usize).div_ceil(16) * 2 * size as usize];
    // SAFETY: the buffers hold `size` rows of 32-bit pixels and of 1-bit word-aligned mask
    // pixels; the bitmaps are copied into the icon and deleted right after.
    unsafe {
        let color = CreateBitmap(side, side, 1, 32, Some(bgra.as_ptr().cast()));
        let monochrome = CreateBitmap(side, side, 1, 1, Some(mask.as_ptr().cast()));
        let info = ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: monochrome,
            hbmColor: color,
        };
        let icon = CreateIconIndirect(&info).ok();
        let _ = DeleteObject(HGDIOBJ(color.0));
        let _ = DeleteObject(HGDIOBJ(monochrome.0));
        icon
    }
}

/// Hears thumbnail button clicks and the taskbar being recreated.
unsafe extern "system" fn subclass(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    // SAFETY: `data` is the `Shared` the subclass was installed with, alive until it's removed.
    let shared = unsafe { &*(data as *const Shared) };
    if message == WM_COMMAND && ((wparam.0 >> 16) & 0xffff) as u32 == THBN_CLICKED {
        let action = match (wparam.0 & 0xffff) as u32 {
            BACK => Some(Action::SeekBy { seconds: -10 }),
            PLAY_PAUSE => Some(Action::TogglePlay),
            FORWARD => Some(Action::SeekBy { seconds: 10 }),
            _ => None,
        };
        if let Some(action) = action {
            let _ = shared.clicks.send(action);
            shared.repaint.request_repaint();
            return LRESULT(0);
        }
    }
    if message == shared.button_created {
        shared.recreated.store(true, Ordering::Relaxed);
        shared.repaint.request_repaint();
    }
    // SAFETY: passes the message on to the window's own procedure.
    unsafe { DefSubclassProc(window, message, wparam, lparam) }
}
