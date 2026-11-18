//! Small Windows helpers: the island's process and exe, opening files and links, the clipboard, the app's
//! single instance, and the event that asks the island to quit.

use di4w_core::log;
use di4w_core::wide::{from_wide, wide};
use std::path::{Path, PathBuf};
use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, GlobalFree, HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Threading::{
    CreateMutexW, EVENT_MODIFY_STATE, OpenEventW, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    QueryFullProcessImageNameW, ReleaseMutex, SetEvent,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowW, GW_OWNER, GetWindow, GetWindowThreadProcessId, IsWindowVisible, SW_RESTORE, SW_SHOWNORMAL,
    SetForegroundWindow, ShowWindow,
};
use windows::core::{BOOL, PCWSTR, PWSTR, w};

/// The window title the single-instance check looks for (not translated).
pub const WINDOW_TITLE: &str = "Dynamic Island Settings";

// ───────────────────────────── processes ─────────────────────────────

/// A running process: its id and exe file name ("DynamicIsland.exe").
pub struct Process {
    pub id: u32,
    pub name: String,
}

pub fn processes() -> Vec<Process> {
    let mut list = Vec::new();
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return list };
        let mut entry = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut ok = Process32FirstW(snapshot, &mut entry).is_ok();
        while ok {
            list.push(Process { id: entry.th32ProcessID, name: from_wide(&entry.szExeFile) });
            ok = Process32NextW(snapshot, &mut entry).is_ok();
        }
        let _ = CloseHandle(snapshot);
    }
    list
}

/// Full path of a process's exe, when it can be read.
pub fn process_path(id: u32) -> Option<PathBuf> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, id).ok()?;
        let mut buffer = [0u16; 1024];
        let mut size = buffer.len() as u32;
        let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buffer.as_mut_ptr()), &mut size).is_ok();
        let _ = CloseHandle(process);
        ok.then(|| PathBuf::from(from_wide(&buffer[..size as usize])))
    }
}

/// The ids of the processes running `exe_name` ("DynamicIsland.exe").
pub fn find_processes(exe_name: &str) -> Vec<u32> {
    processes().into_iter().filter(|p| p.name.eq_ignore_ascii_case(exe_name)).map(|p| p.id).collect()
}

pub fn island_running() -> bool {
    !find_processes("DynamicIsland.exe").is_empty()
}

/// Apps with a window open now (process names without ".exe"), for the quiet-times list.
pub fn open_apps() -> Vec<String> {
    unsafe extern "system" fn add(window: HWND, data: windows::Win32::Foundation::LPARAM) -> BOOL {
        let ids = unsafe { &mut *(data.0 as *mut Vec<u32>) };
        unsafe {
            if IsWindowVisible(window).as_bool() && GetWindow(window, GW_OWNER).map(|o| o.is_invalid()).unwrap_or(true) {
                let mut id = 0u32;
                GetWindowThreadProcessId(window, Some(&mut id));
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
        }
        true.into()
    }
    let mut ids: Vec<u32> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(add), windows::Win32::Foundation::LPARAM(&mut ids as *mut _ as isize));
    }
    let own = std::process::id();
    let all = processes();
    let mut names: Vec<String> = Vec::new();
    for id in ids.into_iter().filter(|id| *id != own) {
        let Some(process) = all.iter().find(|p| p.id == id) else { continue };
        let name = di4w_core::AddonQuiet::normalize(&process.name).to_string();
        let skip = ["DynamicIsland", "explorer", "TextInputHost", "ApplicationFrameHost"];
        if name.is_empty() || skip.iter().any(|s| s.eq_ignore_ascii_case(&name)) {
            continue;
        }
        if !names.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
            names.push(name);
        }
    }
    names.sort_by_key(|n| n.to_lowercase());
    names
}

// ───────────────────────────── the island ─────────────────────────────

/// The folder this exe is in.
pub fn exe_folder() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(Path::to_path_buf)
}

/// Path of DynamicIsland.exe: the one the island recorded, else the one next to (or above) this app.
pub fn island_exe(recorded: Option<&str>) -> Option<PathBuf> {
    if let Some(path) = recorded.map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(path);
    }
    let folder = exe_folder()?;
    [folder.join("..").join("DynamicIsland.exe"), folder.join("DynamicIsland.exe")]
        .into_iter()
        .find(|p| p.is_file())
        .map(|p| std::fs::canonicalize(&p).map(strip_verbatim).unwrap_or(p))
}

/// `\\?\C:\…` (from canonicalize) -> `C:\…`.
pub fn strip_verbatim(path: PathBuf) -> PathBuf {
    match path.to_str().and_then(|p| p.strip_prefix(r"\\?\")) {
        Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest),
        _ => path,
    }
}

/// Opens a file, folder or link with its default app.
pub fn open(target: &str) {
    let target_w = wide(target);
    let result = unsafe { ShellExecuteW(None, w!("open"), PCWSTR(target_w.as_ptr()), None, None, SW_SHOWNORMAL) };
    if result.0 as isize <= 32 {
        log::error(format!("Could not open {target} (error {})", result.0 as isize));
    }
}

// ───────────────────────────── clipboard ─────────────────────────────

pub fn copy_text(text: &str) {
    const CF_UNICODETEXT: u32 = 13;
    let data: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        if OpenClipboard(None).is_err() {
            log::error("Could not open the clipboard");
            return;
        }
        let _ = EmptyClipboard();
        if let Ok(memory) = GlobalAlloc(GMEM_MOVEABLE, data.len() * 2) {
            let target = GlobalLock(memory) as *mut u16;
            if !target.is_null() {
                std::ptr::copy_nonoverlapping(data.as_ptr(), target, data.len());
                let _ = GlobalUnlock(memory);
                if SetClipboardData(CF_UNICODETEXT, Some(HANDLE(memory.0))).is_err() {
                    let _ = GlobalFree(Some(HGLOBAL(memory.0)));
                }
            }
        }
        let _ = CloseClipboard();
    }
}

// ───────────────────────────── single instance ─────────────────────────────

/// The named mutex that keeps this app to one window.
pub struct SingleInstance(HANDLE);

impl SingleInstance {
    /// `None` when another copy already runs (its window is brought forward).
    pub fn acquire() -> Option<Self> {
        // A debug build can run next to the installed app while testing.
        let name = match std::env::var("DI4W_INSTANCE") {
            Ok(suffix) if cfg!(debug_assertions) => format!("DynamicIsland4Windows.Settings.{suffix}"),
            _ => "DynamicIsland4Windows.Settings".to_string(),
        };
        let name_w = wide(&name);
        unsafe {
            let handle = CreateMutexW(None, true, PCWSTR(name_w.as_ptr())).ok()?;
            if GetLastError() == ERROR_ALREADY_EXISTS {
                let _ = CloseHandle(handle);
                bring_existing_window_forward();
                return None;
            }
            Some(Self(handle))
        }
    }

    /// Lets a new copy start (before restarting).
    pub fn release(self) {
        drop(self);
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        unsafe {
            let _ = ReleaseMutex(self.0);
            let _ = CloseHandle(self.0);
        }
    }
}

fn bring_existing_window_forward() {
    let title = wide(WINDOW_TITLE);
    unsafe {
        if let Ok(window) = FindWindowW(None, PCWSTR(title.as_ptr())) {
            let _ = ShowWindow(window, SW_RESTORE);
            let _ = SetForegroundWindow(window);
        }
    }
}

/// Asks a running island to quit cleanly (before its files are replaced).
pub fn signal_island_quit() {
    let name = wide(di4w_core::updates::ISLAND_QUIT_EVENT);
    unsafe {
        if let Ok(event) = OpenEventW(EVENT_MODIFY_STATE, false, PCWSTR(name.as_ptr())) {
            let _ = SetEvent(event);
            let _ = CloseHandle(event);
        }
    }
}

/// Start with Windows, guarded so test runs (DI4W_NO_AUTOSTART) never touch the Run key.
pub fn set_autostart(enabled: bool, island_exe: &Path) {
    if std::env::var_os("DI4W_NO_AUTOSTART").is_some() {
        log::info(format!("Start with Windows would be set to {enabled} (skipped: DI4W_NO_AUTOSTART)"));
        return;
    }
    if let Err(e) = di4w_core::settings::autostart::set(enabled, &island_exe.display().to_string()) {
        log::error_with("Could not change Start with Windows", e);
    }
}

// ───────────────────────────── Windows version ─────────────────────────────

/// A string value under HKEY_LOCAL_MACHINE.
pub fn read_hklm(path: &str, name: &str) -> Option<String> {
    use windows::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
    let (path_w, name_w) = (wide(path), wide(name));
    let mut buffer = [0u16; 512];
    let mut size = (buffer.len() * 2) as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(path_w.as_ptr()),
            PCWSTR(name_w.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr() as *mut _),
            Some(&mut size),
        )
    };
    status.is_ok().then(|| from_wide(&buffer))
}

const CURRENT_VERSION_KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";

pub fn windows_build() -> Option<u32> {
    read_hklm(CURRENT_VERSION_KEY, "CurrentBuild")?.parse().ok()
}

/// "Windows 11 Professional 24H2 (build 26100)".
pub fn windows_version() -> String {
    let build = windows_build().unwrap_or(0);
    let edition = read_hklm(CURRENT_VERSION_KEY, "EditionID").unwrap_or_default();
    let release = read_hklm(CURRENT_VERSION_KEY, "DisplayVersion").unwrap_or_default();
    // ProductName still says "Windows 10" on Windows 11; the build number tells them apart.
    format!("Windows {} {edition} {release} (build {build})", if build >= 22000 { "11" } else { "10" }).replace("  ", " ")
}
