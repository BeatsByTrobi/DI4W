//! "Caps Lock on/off" and "Num Lock on/off". Windows reports key presses to the island through raw input (no
//! global hook, which can slow typing system-wide); only the Caps Lock and Num Lock keys are looked at, and only to
//! then read their on/off state. Nothing else about typing is used or stored.

use crate::addons::hud::Hud;
use crate::addons::{Addon, Base, Context};
use crate::ui::colors;
use di4w_core::{AddonSettings, l};
use std::cell::Cell;
use std::rc::Rc;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyState;
use windows::Win32::UI::Input::*;

const VK_CAPITAL: u16 = 0x14;
const VK_NUMLOCK: u16 = 0x90;
const WM_INPUT: u32 = 0x00FF;
const RI_KEY_BREAK: u16 = 1;

pub struct LockKeys {
    base: Base,
    // Like the iPhone's Silent mode indicator: tinted while the lock is on, plain when it's off.
    on: Hud,
    off: Hud,
    caps: Cell<bool>,
    num: Cell<bool>,
    hook: Cell<Option<u64>>,
}

fn is_on(vk: u16) -> bool {
    unsafe { GetKeyState(vk as i32) & 1 != 0 }
}

/// The virtual key and whether it was released, from a WM_INPUT keyboard message.
fn read_key(raw: isize) -> Option<(u16, bool)> {
    let header_size = std::mem::size_of::<RAWINPUTHEADER>() as u32;
    let mut size = 0u32;
    unsafe {
        GetRawInputData(HRAWINPUT(raw as *mut _), RID_INPUT, None, &mut size, header_size);
        if size == 0 || size > 1024 {
            return None;
        }
        let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
        if GetRawInputData(HRAWINPUT(raw as *mut _), RID_INPUT, Some(buffer.as_mut_ptr().cast()), &mut size, header_size) != size {
            return None;
        }
        let input = &*(buffer.as_ptr() as *const RAWINPUT);
        if input.header.dwType != RIM_TYPEKEYBOARD.0 {
            return None;
        }
        let keyboard = input.data.keyboard;
        Some((keyboard.VKey, keyboard.Flags & RI_KEY_BREAK != 0))
    }
}

impl LockKeys {
    pub fn new(ctx: &Rc<Context>) -> Rc<LockKeys> {
        Rc::new(LockKeys {
            base: Base::new(ctx, "LockKeys", "Lock keys"),
            on: Hud::new("\u{E765}", colors::amber()),
            off: Hud::new("\u{E765}", colors::primary()),
            caps: Cell::new(false),
            num: Cell::new(false),
            hook: Cell::new(None),
        })
    }

    fn check(&self) {
        let (caps, num) = (is_on(VK_CAPITAL), is_on(VK_NUMLOCK));
        if caps != self.caps.get() {
            self.announce("Caps Lock", caps);
        } else if num != self.num.get() {
            self.announce("Num Lock", num);
        }
        self.caps.set(caps);
        self.num.set(num);
    }

    fn announce(&self, key: &str, on: bool) {
        let hud = if on { &self.on } else { &self.off };
        hud.set(None, &if on { l::f("{0} on", &[&key]) } else { l::f("{0} off", &[&key]) }, None);
        self.base.show(hud.alert.clone());
    }
}

impl Addon for LockKeys {
    fn base(&self) -> &Base {
        &self.base
    }

    fn enabled(&self, o: &AddonSettings) -> bool {
        o.lock_keys
    }

    fn start(self: Rc<Self>) -> Result<(), String> {
        self.caps.set(is_on(VK_CAPITAL));
        self.num.set(is_on(VK_NUMLOCK));
        // Keyboard (usage page 1, usage 6), delivered even while another app has focus.
        let device = RAWINPUTDEVICE { usUsagePage: 1, usUsage: 6, dwFlags: RIDEV_INPUTSINK, hwndTarget: self.base.ctx.island.hwnd() };
        unsafe { RegisterRawInputDevices(&[device], std::mem::size_of::<RAWINPUTDEVICE>() as u32) }.map_err(|e| format!("Keyboard input registration refused ({})", e.code()))?;
        let weak = Rc::downgrade(&self);
        let id = self.base.ctx.island.add_hook(move |msg, _, lparam| {
            if msg != WM_INPUT {
                return;
            }
            let Some((key, released)) = read_key(lparam.0) else { return };
            if !released || (key != VK_CAPITAL && key != VK_NUMLOCK) {
                return;
            }
            // Windows flips the toggle state as the key goes down; read it once this message has been handled.
            let w = weak.clone();
            crate::rt::defer(move || {
                if let Some(k) = w.upgrade() {
                    k.check();
                }
            });
        });
        self.hook.set(Some(id));
        Ok(())
    }

    fn stop(&self) {
        if let Some(id) = self.hook.take() {
            self.base.ctx.island.remove_hook(id);
        }
        let device = RAWINPUTDEVICE { usUsagePage: 1, usUsage: 6, dwFlags: RIDEV_REMOVE, hwndTarget: HWND::default() };
        unsafe {
            let _ = RegisterRawInputDevices(&[device], std::mem::size_of::<RAWINPUTDEVICE>() as u32);
        }
    }

    fn preview(self: Rc<Self>) {
        self.announce("Caps Lock", true);
    }
}
