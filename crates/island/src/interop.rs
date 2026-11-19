//! Small Win32 helpers: fullscreen detection, processes and their windows, known folders, opening links.

use crate::rt::WideNull;
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow};
use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS};
use windows::Win32::System::Threading::{
    CREATE_NO_WINDOW, CreateProcessW, OpenProcess, PROCESS_INFORMATION, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
    STARTUPINFOW,
};
use windows::Win32::UI::Shell::{
    KNOWN_FOLDER_FLAG, QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN, SHGetKnownFolderPath, SHQueryUserNotificationState, ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, GUID, PCWSTR, PWSTR, w};

// Screenshot/overlay tools that cover the whole screen for a moment. They must never count as "fullscreen app".
const OVERLAY_PROCESSES: [&str; 17] = [
    "ScreenClippingHost", "SnippingTool", "ScreenSketch", "ShareX", "Greenshot", "Lightshot", "Flameshot",
    "PowerToys.MeasureTool", "PowerToys.ColorPickerUI", "PowerToys.ScreenRuler", "explorer",
    "ShellExperienceHost", "StartMenuExperienceHost", "SearchHost", "SearchApp", "LockApp", "DynamicIsland",
];

const SHELL_CLASSES: [&str; 4] = ["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd"];

/// True when a real fullscreen app (game, video, presentation) is in front. Screenshot overlays, the desktop and
/// shell pop-ups are ignored, so taking a screenshot never hides the island.
pub fn is_fullscreen_app_running() -> bool {
    unsafe {
        let Ok(state) = SHQueryUserNotificationState() else { return false };
        if state == QUNS_RUNNING_D3D_FULL_SCREEN || state == QUNS_PRESENTATION_MODE {
            return !is_overlay(GetForegroundWindow());
        }
        if state != QUNS_BUSY {
            return false;
        }
        // "Busy" is reported for any topmost fullscreen window, including the snipping overlay. Check it ourselves.
        let fg = GetForegroundWindow();
        if fg.is_invalid() || is_overlay(fg) {
            return false;
        }
        let mut r = RECT::default();
        if GetWindowRect(fg, &mut r).is_err() {
            return false;
        }
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST), &mut info).as_bool() {
            return false;
        }
        let m = info.rcMonitor;
        r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom
    }
}

fn is_overlay(hwnd: HWND) -> bool {
    if hwnd.is_invalid() {
        return true;
    }
    let class = class_name(hwnd);
    if SHELL_CLASSES.contains(&class.as_str()) {
        return true;
    }
    process_name_of_window(hwnd).is_some_and(|name| OVERLAY_PROCESSES.iter().any(|p| p.eq_ignore_ascii_case(&name)))
}

pub fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetClassNameW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

pub fn window_text(hwnd: HWND) -> String {
    let mut buf = [0u16; 512];
    let n = unsafe { GetWindowTextW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

/// The process name ("obs64", without ".exe") owning a window.
pub fn process_name_of_window(hwnd: HWND) -> Option<String> {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    if pid == 0 {
        return None;
    }
    process_name(pid)
}

/// A process's name (file name without ".exe"), or `None` if it can't be read (it closed, or it's protected).
pub fn process_name(pid: u32) -> Option<String> {
    let path = process_path(pid)?;
    let file = std::path::Path::new(&path).file_stem()?.to_string_lossy().to_string();
    Some(file)
}

pub fn process_path(pid: u32) -> Option<String> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(process);
        ok.then(|| String::from_utf16_lossy(&buf[..len as usize]))
    }
}

/// Process name of the app in front ("obs64"), or `None`.
pub fn foreground_process_name() -> Option<String> {
    process_name_of_window(unsafe { GetForegroundWindow() })
}

/// Every running process: (pid, name without ".exe").
pub fn processes() -> Vec<(u32, String)> {
    let mut out = Vec::new();
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
        let mut entry = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut ok = Process32FirstW(snapshot, &mut entry).is_ok();
        while ok {
            let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            let file = String::from_utf16_lossy(&entry.szExeFile[..len]);
            let name = file.strip_suffix(".exe").or_else(|| file.strip_suffix(".EXE")).unwrap_or(&file).to_string();
            out.push((entry.th32ProcessID, name));
            ok = Process32NextW(snapshot, &mut entry).is_ok();
        }
        let _ = CloseHandle(snapshot);
    }
    out
}

/// True when a process with this name (no ".exe", any case) runs.
pub fn is_running(name: &str) -> bool {
    processes().iter().any(|(_, n)| n.eq_ignore_ascii_case(name))
}

/// The main window of a process: visible, top-level, not owned by another window.
fn main_window(pid: u32) -> Option<HWND> {
    struct Search {
        pid: u32,
        found: Option<HWND>,
    }
    unsafe extern "system" fn each(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let search = unsafe { &mut *(lparam.0 as *mut Search) };
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        let owned = unsafe { GetWindow(hwnd, GW_OWNER) }.is_ok_and(|o| !o.is_invalid());
        if pid == search.pid && unsafe { IsWindowVisible(hwnd) }.as_bool() && !owned {
            search.found = Some(hwnd);
            return false.into();
        }
        true.into()
    }
    let mut search = Search { pid, found: None };
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut search as *mut _ as isize));
    }
    search.found
}

/// Brings the main window of the first process with the given name to the front; false if none has one.
pub fn focus_process(name: &str) -> bool {
    for (pid, n) in processes() {
        if !n.eq_ignore_ascii_case(name) {
            continue;
        }
        let Some(hwnd) = main_window(pid) else { continue };
        unsafe {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            let _ = SetForegroundWindow(hwnd);
        }
        return true;
    }
    false
}

/// A known folder's path (Downloads, Screenshots).
pub fn known_folder(id: &GUID) -> Option<std::path::PathBuf> {
    unsafe {
        let p = SHGetKnownFolderPath(id, KNOWN_FOLDER_FLAG(0), None).ok()?;
        let path = p.to_string().ok();
        windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as *const _));
        path.map(std::path::PathBuf::from)
    }
}

/// Opens a file, folder or link with its default app (Process.Start with UseShellExecute).
pub fn shell_open(target: &str) {
    let file = target.encode_wide_null();
    unsafe {
        ShellExecuteW(None, w!("open"), PCWSTR(file.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
    }
}

/// Starts a program with arguments (already quoted as needed).
pub fn run(exe: &str, args: &str, working_dir: Option<&str>, hidden: bool) -> bool {
    let mut command = format!("\"{exe}\"");
    if !args.is_empty() {
        command.push(' ');
        command.push_str(args);
    }
    let mut command = command.encode_wide_null();
    let dir = working_dir.map(|d| d.encode_wide_null());
    let startup = STARTUPINFOW { cb: std::mem::size_of::<STARTUPINFOW>() as u32, ..Default::default() };
    let mut info = PROCESS_INFORMATION::default();
    unsafe {
        let ok = CreateProcessW(
            PCWSTR::null(),
            Some(PWSTR(command.as_mut_ptr())),
            None,
            None,
            false,
            if hidden { CREATE_NO_WINDOW } else { Default::default() },
            None,
            dir.as_ref().map_or(PCWSTR::null(), |d| PCWSTR(d.as_ptr())),
            &startup,
            &mut info,
        )
        .is_ok();
        if ok {
            let _ = CloseHandle(info.hProcess);
            let _ = CloseHandle(info.hThread);
        } else {
            di4w_core::log::error(format!("Could not start {exe} {args}"));
        }
        ok
    }
}

/// A command-line argument quoted the way Windows programs read them back.
pub fn quote_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_string();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.push_str(&"\\".repeat(backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.push_str(&"\\".repeat(backslashes * 2));
    out.push('"');
    out
}

/// Shows "explorer /select" for a file.
pub fn show_in_folder(path: &std::path::Path) {
    run("explorer.exe", &format!("/select,\"{}\"", path.display()), None, false);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_arguments() {
        assert_eq!(quote_arg("--page"), "--page");
        assert_eq!(quote_arg("About Us"), "\"About Us\"");
        assert_eq!(quote_arg(""), "\"\"");
        assert_eq!(quote_arg("a\"b"), "\"a\\\"b\"");
    }

    #[test]
    fn lists_this_process() {
        let me = std::process::id();
        assert!(processes().iter().any(|(pid, _)| *pid == me));
    }
}
