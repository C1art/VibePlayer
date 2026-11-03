//! The app icon in the chosen color theme. The exe carries one icon per theme, with resource
//! IDs from 1 in `ColorTheme::ALL` order (see build.rs); this points the window and the
//! shell (VibePlayer's shortcuts and the videos it opens) at the right one.

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::thread;

use library::ColorTheme;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoTaskMemFree, CoUninitialize, IPersistFile, STGM_READWRITE,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::{
    HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegGetValueW, RegSetKeyValueW,
};
use windows::Win32::UI::Shell::{
    FOLDERID_CommonPrograms, FOLDERID_Desktop, FOLDERID_Programs, FOLDERID_PublicDesktop,
    FOLDERID_RoamingAppData, IShellLinkW, KF_FLAG_DEFAULT, SHCNE_ASSOCCHANGED, SHCNF_IDLIST,
    SHChangeNotify, SHGetKnownFolderPath, ShellLink,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, ICON_BIG, ICON_SMALL, IMAGE_ICON, LR_SHARED, LoadImageW, SM_CXICON,
    SM_CXSMICON, SendMessageW, WM_SETICON,
};
use windows::core::{GUID, Interface, PCWSTR, w};

const PROG_ID: PCWSTR = w!("VibePlayer.Video\\DefaultIcon");
const USER_PROG_ID: PCWSTR = w!("Software\\Classes\\VibePlayer.Video\\DefaultIcon");

fn resource_id(theme: ColorTheme) -> u16 {
    let index = ColorTheme::ALL
        .iter()
        .position(|&t| t == theme)
        .unwrap_or(0);
    u16::try_from(index + 1).unwrap_or(1)
}

/// Gives `window` the theme's icons now, and updates the shell's on a worker.
pub fn show(window: isize, theme: ColorTheme) {
    let id = resource_id(theme);
    set_window_icons(window, id);
    let _ = thread::Builder::new()
        .name("shell icon".into())
        .spawn(move || update_shell(id));
}

/// The large icon is the taskbar's; the small one the title bar's.
fn set_window_icons(window: isize, id: u16) {
    // SAFETY: the icons are shared resources of our own module, loaded at standard sizes and
    // never destroyed; `window` is our own top-level window.
    unsafe {
        let Ok(module) = GetModuleHandleW(None) else {
            return;
        };
        for (which, metric) in [(ICON_BIG, SM_CXICON), (ICON_SMALL, SM_CXSMICON)] {
            let size = GetSystemMetrics(metric);
            let name = PCWSTR(usize::from(id) as *const u16);
            if let Ok(icon) =
                LoadImageW(Some(module.into()), name, IMAGE_ICON, size, size, LR_SHARED)
            {
                SendMessageW(
                    HWND(window as *mut c_void),
                    WM_SETICON,
                    Some(WPARAM(which as usize)),
                    Some(LPARAM(icon.0 as isize)),
                );
            }
        }
    }
}

fn update_shell(id: u16) {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let location = format!("\"{}\",-{id}", exe.display());
    let mut changed = set_file_icon(&location);
    // SAFETY: COM is set up for this worker thread alone and torn down at its end.
    if unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok() {
        for link in shortcuts() {
            changed |= retarget(&link, &exe, id).unwrap_or(false);
        }
        // SAFETY: matches the successful CoInitializeEx above.
        unsafe { CoUninitialize() };
    }
    if changed {
        // SAFETY: a notification without item pointers.
        unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
    }
}

/// The icon videos opened with VibePlayer show, if the installer registered them.
fn set_file_icon(location: &str) -> bool {
    let current = read_string(HKEY_CLASSES_ROOT, PROG_ID);
    if current.as_deref().is_none_or(|value| value == location) {
        return false;
    }
    let data: Vec<u16> = location.encode_utf16().chain([0]).collect();
    // SAFETY: `data` is a NUL-terminated wide string, and its byte length is passed with it.
    unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            USER_PROG_ID,
            PCWSTR::null(),
            REG_SZ.0,
            Some(data.as_ptr().cast()),
            u32::try_from(data.len() * 2).unwrap_or(0),
        )
    }
    .is_ok()
}

fn read_string(root: windows::Win32::System::Registry::HKEY, key: PCWSTR) -> Option<String> {
    let mut buffer = [0u16; 1024];
    let mut size = u32::try_from(buffer.len() * 2).ok()?;
    // SAFETY: the buffer and its size in bytes are passed together.
    unsafe {
        RegGetValueW(
            root,
            key,
            PCWSTR::null(),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut size),
        )
    }
    .ok()
    .ok()?;
    let length = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..length]))
}

/// The installer's Start menu and desktop shortcuts, and any pinned to the taskbar.
fn shortcuts() -> Vec<PathBuf> {
    let mut links: Vec<PathBuf> = [
        FOLDERID_Programs,
        FOLDERID_CommonPrograms,
        FOLDERID_Desktop,
        FOLDERID_PublicDesktop,
    ]
    .iter()
    .filter_map(known_folder)
    .map(|folder| folder.join("VibePlayer.lnk"))
    .collect();
    if let Some(pinned) = known_folder(&FOLDERID_RoamingAppData)
        .map(|data| data.join(r"Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar"))
        && let Ok(entries) = std::fs::read_dir(pinned)
    {
        links.extend(entries.flatten().map(|entry| entry.path()).filter(|path| {
            path.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("lnk"))
        }));
    }
    links.retain(|link| link.exists());
    links
}

fn known_folder(id: &GUID) -> Option<PathBuf> {
    // SAFETY: the returned string is copied, then freed with CoTaskMemFree as required.
    unsafe {
        let path = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        let folder = path.to_string().ok().map(PathBuf::from);
        CoTaskMemFree(Some(path.0 as *const c_void));
        folder
    }
}

/// Points a shortcut to `exe` at icon `id`. Returns whether it changed; shortcuts to anything
/// else are left alone.
fn retarget(link: &Path, exe: &Path, id: u16) -> windows::core::Result<bool> {
    let wide = |path: &Path| -> Vec<u16> { path.as_os_str().encode_wide().chain([0]).collect() };
    let link_path = wide(link);
    let exe_path = wide(exe);
    // SAFETY: COM is initialized on this thread; every buffer outlives the call using it.
    unsafe {
        let shell_link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        let file: IPersistFile = shell_link.cast()?;
        file.Load(PCWSTR(link_path.as_ptr()), STGM_READWRITE)?;
        let mut target = [0u16; 1024];
        shell_link.GetPath(&mut target, std::ptr::null_mut(), 0)?;
        let target =
            String::from_utf16_lossy(&target[..target.iter().position(|&c| c == 0).unwrap_or(0)]);
        if !target.eq_ignore_ascii_case(&exe.to_string_lossy()) {
            return Ok(false);
        }
        let mut icon = [0u16; 1024];
        let mut index = 0;
        shell_link.GetIconLocation(&mut icon, &mut index)?;
        let icon =
            String::from_utf16_lossy(&icon[..icon.iter().position(|&c| c == 0).unwrap_or(0)]);
        let wanted = -i32::from(id);
        if index == wanted && icon.eq_ignore_ascii_case(&exe.to_string_lossy()) {
            return Ok(false);
        }
        shell_link.SetIconLocation(PCWSTR(exe_path.as_ptr()), wanted)?;
        file.Save(PCWSTR::null(), true)?;
    }
    Ok(true)
}
