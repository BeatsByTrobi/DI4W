//! USB stick / external drive plugged in or removed, with its name and size. Click opens it.

use crate::addons::{Addon, Base, Context};
use crate::model::Alert;
use crate::rt::WideNull;
use crate::ui::colors;
use crate::ui::{self, ICON_SIZE};
use di4w_core::{AddonSettings, l};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;
use windows::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetLogicalDrives, GetVolumeInformationW};
use windows::core::PCWSTR;

const WM_DEVICECHANGE: u32 = 0x0219;
const DBT_DEVICEARRIVAL: usize = 0x8000;
const DBT_DEVICEREMOVECOMPLETE: usize = 0x8004;
const DBT_DEVTYP_VOLUME: u32 = 2;
const DBTF_NET: u16 = 2;

pub struct UsbDrives {
    base: Base,
    names: RefCell<HashMap<char, String>>,
    hook: Cell<Option<u64>>,
}

/// "1.5 GB": sizes the way Windows Explorer shows them (1024-based, one decimal, in the user's number format).
pub fn format_size(bytes: i64) -> String {
    format_size_with(bytes, &crate::activities::decimal_separator())
}

fn format_size_with(bytes: i64, separator: &str) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        return format!("{bytes} B");
    }
    // .NET's "0.#": one decimal, dropped when it's zero.
    let rounded = (value * 10.0).round() / 10.0;
    let text = if rounded.fract() == 0.0 { format!("{rounded:.0}") } else { format!("{rounded:.1}").replace('.', separator) };
    format!("{text} {}", UNITS[unit])
}

/// The drive's label (`None` if it isn't ready) and total size.
fn drive_info(letter: char) -> Option<(String, i64)> {
    let root = format!("{letter}:\\").as_str().encode_wide_null();
    let mut label = [0u16; 261];
    unsafe {
        GetVolumeInformationW(PCWSTR(root.as_ptr()), Some(&mut label), None, None, None, None).ok()?;
        let mut total = 0u64;
        let _ = GetDiskFreeSpaceExW(PCWSTR(root.as_ptr()), None, Some(&mut total), None);
        let len = label.iter().position(|&c| c == 0).unwrap_or(0);
        Some((String::from_utf16_lossy(&label[..len]), total as i64))
    }
}

impl UsbDrives {
    pub fn new(ctx: &Rc<Context>) -> Rc<UsbDrives> {
        Rc::new(UsbDrives { base: Base::new(ctx, "UsbDrives", "USB drives"), names: RefCell::new(HashMap::new()), hook: Cell::new(None) })
    }

    fn remember(&self, letter: char) -> String {
        let name = match drive_info(letter) {
            Some((label, _)) if !label.trim().is_empty() => label,
            _ => l::f("Drive {0}:", &[&letter]),
        };
        self.names.borrow_mut().insert(letter, name.clone());
        name
    }

    fn arrived(&self, letter: char) {
        let Some((_, total)) = drive_info(letter) else {
            di4w_core::log::info(format!("Drive {letter}: not readable yet"));
            return;
        };
        let name = self.remember(letter);
        let size = if total > 0 { format!(" · {}", format_size(total)) } else { String::new() };
        let root = format!("{letter}:\\");
        self.announce(name, l::f("Connected ({0}:)", &[&letter]) + &size, true, Some(Rc::new(move || {
            crate::interop::run("explorer.exe", &format!("\"{root}\""), None, false);
        })));
    }

    fn removed(&self, letter: char) {
        let name = self.names.borrow_mut().remove(&letter).unwrap_or_else(|| l::f("Drive {0}:", &[&letter]));
        self.announce(name, l::t("Removed").into(), false, None);
    }

    fn announce(&self, name: String, subtitle: String, connected: bool, on_click: Option<Rc<dyn Fn()>>) {
        let alert = Alert::new(move || ui::alert_view(ui::glyph_circle("\u{E88E}", if connected { colors::teal() } else { colors::button_bg() }, ICON_SIZE, 22.0), &name, &subtitle, None, None))
            .seconds(3.5)
            .on_click_opt(on_click);
        self.base.show(alert.rc());
    }
}

impl Addon for UsbDrives {
    fn base(&self) -> &Base {
        &self.base
    }

    fn enabled(&self, o: &AddonSettings) -> bool {
        o.usb_drives
    }

    fn start(self: Rc<Self>) -> Result<(), String> {
        let mask = unsafe { GetLogicalDrives() };
        for i in 0..26 {
            if mask & (1 << i) != 0 {
                self.remember((b'A' + i as u8) as char);
            }
        }
        let weak = Rc::downgrade(&self);
        let id = self.base.ctx.island.add_hook(move |msg, wparam, lparam| {
            if msg != WM_DEVICECHANGE || lparam.0 == 0 {
                return;
            }
            let event = wparam.0;
            if event != DBT_DEVICEARRIVAL && event != DBT_DEVICEREMOVECOMPLETE {
                return;
            }
            // DEV_BROADCAST_VOLUME: size, device type, reserved, unit mask, flags.
            let header = unsafe { std::slice::from_raw_parts(lparam.0 as *const u8, 18) };
            let read32 = |o: usize| u32::from_le_bytes([header[o], header[o + 1], header[o + 2], header[o + 3]]);
            if read32(4) != DBT_DEVTYP_VOLUME {
                return;
            }
            let unit_mask = read32(12);
            let flags = u16::from_le_bytes([header[16], header[17]]);
            if flags & DBTF_NET != 0 {
                return; // mapped network drives
            }
            for i in 0..26 {
                if unit_mask & (1 << i) == 0 {
                    continue;
                }
                let letter = (b'A' + i as u8) as char;
                let w = weak.clone();
                if event == DBT_DEVICEARRIVAL {
                    // Let Windows finish mounting before reading the label and size.
                    crate::rt::after(Duration::from_millis(700), move || {
                        if let Some(u) = w.upgrade() {
                            u.arrived(letter);
                        }
                    });
                } else if let Some(u) = w.upgrade() {
                    u.removed(letter);
                }
            }
        });
        self.hook.set(Some(id));
        Ok(())
    }

    fn stop(&self) {
        if let Some(id) = self.hook.take() {
            self.base.ctx.island.remove_hook(id);
        }
    }

    fn preview(self: Rc<Self>) {
        self.announce(l::t("USB drive").into(), l::f("Connected ({0}:)", &[&'E']) + &format!(" · {}", format_size(64 * 1024 * 1024 * 1024)), true, None);
    }
}

#[cfg(test)]
mod tests {
    use super::format_size_with as size;

    #[test]
    fn sizes_like_explorer() {
        assert_eq!(size(512, "."), "512 B");
        assert_eq!(size(2_400_000, "."), "2.3 MB");
        assert_eq!(size(64 * 1024 * 1024 * 1024, "."), "64 GB");
        assert_eq!(size(1536, ","), "1,5 KB");
    }
}
