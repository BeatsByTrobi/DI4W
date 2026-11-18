//! Tooltips for the island's buttons and icons (WPF's `ToolTip`): shown after the mouse rests on something for a
//! moment, under the cursor, and hidden after a few seconds or when the mouse moves on.

use crate::rt::{Timer, WideNull};
use std::cell::RefCell;
use std::time::Duration;
use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::PWSTR;

struct Tip {
    window: HWND,
    owner: HWND,
    text: Option<String>,
    show: Option<Timer>,
    hide: Option<Timer>,
    buffer: Vec<u16>,
}

thread_local! {
    static TIP: RefCell<Option<Tip>> = const { RefCell::new(None) };
}

fn info(tip: &mut Tip) -> TTTOOLINFOW {
    TTTOOLINFOW {
        cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
        uFlags: TTF_TRACK | TTF_ABSOLUTE,
        hwnd: tip.owner,
        uId: 1,
        lpszText: PWSTR(tip.buffer.as_mut_ptr()),
        ..Default::default()
    }
}

fn ensure(owner: HWND) {
    TIP.with(|t| {
        if t.borrow().as_ref().is_some_and(|tip| tip.owner == owner) {
            return;
        }
        let window = unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                TOOLTIPS_CLASSW,
                None,
                WINDOW_STYLE(WS_POPUP.0 | TTS_NOPREFIX | TTS_ALWAYSTIP),
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                Some(owner),
                None,
                None,
                None,
            )
            .unwrap_or_default()
        };
        let mut tip = Tip { window, owner, text: None, show: None, hide: None, buffer: " ".encode_wide_null() };
        let mut ti = info(&mut tip);
        unsafe {
            SendMessageW(window, TTM_ADDTOOLW, None, Some(LPARAM(&mut ti as *mut _ as isize)));
            SendMessageW(window, TTM_SETMAXTIPWIDTH, None, Some(LPARAM(400)));
        }
        *t.borrow_mut() = Some(tip);
    });
}

/// The mouse is over something with this tooltip (or nothing). Call on every mouse move.
pub fn hover(owner: HWND, text: Option<String>) {
    ensure(owner);
    TIP.with(|t| {
        let mut t = t.borrow_mut();
        let Some(tip) = t.as_mut() else { return };
        if tip.text == text {
            return;
        }
        tip.text = text.clone();
        tip.show = None;
        tip.hide = None;
        let mut ti = info(tip);
        unsafe {
            SendMessageW(tip.window, TTM_TRACKACTIVATE, Some(WPARAM(0)), Some(LPARAM(&mut ti as *mut _ as isize)));
        }
        if text.is_some() {
            tip.show = Some(Timer::once(Duration::from_millis(500), show));
        }
    });
}

fn show() {
    TIP.with(|t| {
        let mut t = t.borrow_mut();
        let Some(tip) = t.as_mut() else { return };
        let Some(text) = tip.text.clone() else { return };
        tip.buffer = text.as_str().encode_wide_null();
        let mut ti = info(tip);
        let mut cursor = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut cursor);
            SendMessageW(tip.window, TTM_UPDATETIPTEXTW, None, Some(LPARAM(&mut ti as *mut _ as isize)));
            let pos = ((cursor.y + 20) as u32 as isize) << 16 | (cursor.x as u32 as isize & 0xFFFF);
            SendMessageW(tip.window, TTM_TRACKPOSITION, None, Some(LPARAM(pos)));
            SendMessageW(tip.window, TTM_TRACKACTIVATE, Some(WPARAM(1)), Some(LPARAM(&mut ti as *mut _ as isize)));
        }
        tip.hide = Some(Timer::once(Duration::from_secs(5), || {
            TIP.with(|t| {
                if let Some(tip) = t.borrow_mut().as_mut() {
                    let mut ti = info(tip);
                    unsafe {
                        SendMessageW(tip.window, TTM_TRACKACTIVATE, Some(WPARAM(0)), Some(LPARAM(&mut ti as *mut _ as isize)));
                    }
                }
            });
        }));
    });
}
