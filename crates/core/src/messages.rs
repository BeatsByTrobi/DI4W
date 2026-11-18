//! Small commands from the settings app (or a demo script) to the running island, sent as `WM_COPYDATA` to the
//! island's window, which is titled "Dynamic Island" and owned by a `DynamicIsland.exe` process.
//! Only `preview:<add-on>` for now, which shows a sample pop-up.

use crate::wide::{from_wide, wide};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, WPARAM};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, GetWindowThreadProcessId, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_COPYDATA};
use windows::core::{PCWSTR, PWSTR};

/// Marks our messages, so a stray `WM_COPYDATA` from another program is ignored ("DI4W").
pub const TAG: usize = 0x4449_3457;

pub const PREVIEW_PREFIX: &str = "preview:";

/// The island window's title (also how other tools find it).
pub const ISLAND_TITLE: &str = "Dynamic Island";

/// Sends the command; false if the island isn't running (or didn't answer within a second).
pub fn send(text: &str) -> bool {
    let Some(window) = find_island() else { return false };
    let payload = wide(text); // NUL-terminated
    let data = COPYDATASTRUCT {
        dwData: TAG,
        cbData: (payload.len() * 2) as u32,
        lpData: payload.as_ptr() as *mut _,
    };
    let mut result = 0usize;
    unsafe {
        SendMessageTimeoutW(
            window,
            WM_COPYDATA,
            WPARAM(0),
            LPARAM(&data as *const _ as isize),
            SMTO_ABORTIFHUNG,
            1000,
            Some(&mut result),
        )
        .0 != 0
    }
}

/// The text of one of our `WM_COPYDATA` messages (its `lParam`), or `None` for anything else.
///
/// # Safety
/// `lparam` must be 0 or point to a `COPYDATASTRUCT`, as Windows passes it with `WM_COPYDATA`.
pub unsafe fn read(lparam: LPARAM) -> Option<String> {
    if lparam.0 == 0 {
        return None;
    }
    let data = unsafe { &*(lparam.0 as *const COPYDATASTRUCT) };
    if data.dwData != TAG || data.lpData.is_null() || data.cbData == 0 || data.cbData > 4096 {
        return None;
    }
    let units = unsafe { std::slice::from_raw_parts(data.lpData as *const u16, data.cbData as usize / 2) };
    Some(from_wide(units))
}

/// The island's own window: titled "Dynamic Island" and owned by a DynamicIsland.exe process.
pub fn find_island() -> Option<HWND> {
    let title = wide(ISLAND_TITLE);
    let mut after: Option<HWND> = None;
    loop {
        let window = unsafe { FindWindowExW(None, after, PCWSTR::null(), PCWSTR(title.as_ptr())) }.ok()?;
        if is_island_process(window) {
            return Some(window);
        }
        after = Some(window);
    }
}

fn is_island_process(window: HWND) -> bool {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(window, Some(&mut pid)) };
    let Ok(process) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }) else { return false };
    let mut buffer = [0u16; 1024];
    let mut size = buffer.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buffer.as_mut_ptr()), &mut size) }.is_ok();
    unsafe {
        let _ = CloseHandle(process);
    }
    ok && std::path::Path::new(&from_wide(&buffer[..size as usize]))
        .file_stem()
        .is_some_and(|stem| stem.eq_ignore_ascii_case("DynamicIsland"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_copy_data(tag: usize, text: &str) -> Option<String> {
        let payload: Vec<u16> = text.encode_utf16().collect();
        let data = COPYDATASTRUCT { dwData: tag, cbData: (payload.len() * 2) as u32, lpData: payload.as_ptr() as *mut _ };
        unsafe { read(LPARAM(&data as *const _ as isize)) }
    }

    #[test]
    fn reads_a_tagged_message() {
        assert_eq!(with_copy_data(TAG, "preview:Battery\0").as_deref(), Some("preview:Battery"));
    }

    #[test]
    fn ignores_messages_from_other_programs() {
        assert_eq!(with_copy_data(1234, "preview:Battery\0"), None);
        assert_eq!(unsafe { read(LPARAM(0)) }, None);
    }
}
