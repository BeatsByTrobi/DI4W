//! Records a global keyboard shortcut. Recording uses a low-level keyboard hook: shortcuts another app (or the
//! island) has registered never reach a normal key handler, because Windows delivers them to that app first.
//! The hook sees them before that and swallows the recorded combo, so the other app isn't triggered.

use di4w_core::hotkey;
use std::cell::RefCell;
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, HOT_KEY_MODIFIERS, RegisterHotKey, UnregisterHotKey};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, HHOOK, KBDLLHOOKSTRUCT, SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_KEYDOWN, WM_SYSKEYDOWN,
};

/// How recording ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// A combination with Ctrl, Alt or Win: `(modifiers, key)` as `RegisterHotKey` takes them.
    Recorded(u32, u32),
    /// Esc.
    Cancelled,
}

type Done = Box<dyn Fn(Outcome)>;

thread_local! {
    static HOOK: RefCell<Option<HHOOK>> = const { RefCell::new(None) };
    static DONE: RefCell<Option<Done>> = const { RefCell::new(None) };
}

/// Starts listening; `done` runs (on the UI thread) once a shortcut was pressed or Esc cancelled.
pub fn start(done: impl Fn(Outcome) + 'static) {
    stop();
    DONE.with(|d| *d.borrow_mut() = Some(Box::new(done)));
    unsafe {
        let module = GetModuleHandleW(None).ok();
        match SetWindowsHookExW(WH_KEYBOARD_LL, Some(on_key), module.map(Into::into), 0) {
            Ok(hook) => HOOK.with(|h| *h.borrow_mut() = Some(hook)),
            Err(e) => di4w_core::log::error_with("Could not listen for the shortcut", e),
        }
    }
}

pub fn stop() {
    HOOK.with(|h| {
        if let Some(hook) = h.borrow_mut().take() {
            unsafe {
                let _ = UnhookWindowsHookEx(hook);
            }
        }
    });
}

pub fn is_recording() -> bool {
    HOOK.with(|h| h.borrow().is_some())
}

/// Tries to claim the shortcut for a moment; if another app already has it, this fails.
pub fn is_free(modifiers: u32, key: u32) -> bool {
    const PROBE_ID: i32 = 0x7F00;
    unsafe {
        if RegisterHotKey(None, PROBE_ID, HOT_KEY_MODIFIERS(modifiers), key).is_err() {
            return false;
        }
        let _ = UnregisterHotKey(None, PROBE_ID);
    }
    true
}

fn is_down(vk: i32) -> bool {
    unsafe { (GetAsyncKeyState(vk) as u16 & 0x8000) != 0 }
}

/// Shift/Ctrl/Alt (generic and left/right), Win keys and Caps Lock.
fn is_modifier_key(key: u32) -> bool {
    matches!(key, 0x10 | 0x11 | 0x12 | 0x14 | 0x5B | 0x5C | 0xA0..=0xA5)
}

/// What a key press means while recording; `None` lets it through.
pub fn interpret(key: u32, modifiers: u32) -> Option<Outcome> {
    const VK_ESCAPE: u32 = 0x1B;
    if key == VK_ESCAPE && modifiers == 0 {
        return Some(Outcome::Cancelled);
    }
    // Wait for a real key; a shortcut needs Ctrl, Alt or Win so it doesn't steal normal typing.
    if is_modifier_key(key) || modifiers & !hotkey::MOD_SHIFT == 0 {
        return None;
    }
    Some(Outcome::Recorded(modifiers, key))
}

unsafe extern "system" fn on_key(code: i32, message: WPARAM, data: LPARAM) -> LRESULT {
    if code >= 0 && (message.0 as u32 == WM_KEYDOWN || message.0 as u32 == WM_SYSKEYDOWN) {
        let key = unsafe { (*(data.0 as *const KBDLLHOOKSTRUCT)).vkCode };
        let mut modifiers = 0;
        if is_down(0x11) {
            modifiers |= hotkey::MOD_CONTROL;
        }
        if is_down(0x12) {
            modifiers |= hotkey::MOD_ALT;
        }
        if is_down(0x10) {
            modifiers |= hotkey::MOD_SHIFT;
        }
        if is_down(0x5B) || is_down(0x5C) {
            modifiers |= hotkey::MOD_WIN;
        }
        if let Some(outcome) = interpret(key, modifiers) {
            let _ = slint::invoke_from_event_loop(move || {
                stop();
                if let Some(done) = DONE.with(|d| d.borrow_mut().take()) {
                    done(outcome);
                }
            });
            return LRESULT(1); // swallow
        }
    }
    unsafe { CallNextHookEx(None, code, message, data) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn needs_ctrl_alt_or_win() {
        assert_eq!(interpret(0x49, hotkey::MOD_CONTROL | hotkey::MOD_ALT), Some(Outcome::Recorded(3, 0x49)));
        assert_eq!(interpret(0x49, hotkey::MOD_SHIFT), None);
        assert_eq!(interpret(0x49, 0), None);
        assert_eq!(interpret(0x11, hotkey::MOD_CONTROL), None); // still holding modifiers
        assert_eq!(interpret(0x1B, 0), Some(Outcome::Cancelled));
        assert_eq!(interpret(0x1B, hotkey::MOD_CONTROL), Some(Outcome::Recorded(hotkey::MOD_CONTROL, 0x1B)));
    }
}
