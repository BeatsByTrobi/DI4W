//! The displays the island can sit on: the island positions itself on one, the settings app lists them.
//! The setting [`crate::AppSettings::monitor`] is "" (main display), "Mouse" (follow the mouse) or a device name
//! such as `\\.\DISPLAY2`.

use crate::l;
use crate::wide::{from_wide, wide};
use windows::Win32::Foundation::{LPARAM, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    DISPLAY_DEVICEW, EnumDisplayDevicesW, EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
    MONITORINFOEXW,
};
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
use windows::core::{BOOL, PCWSTR};

pub const FOLLOW_MOUSE: &str = "Mouse";

/// A rectangle in physical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn width(&self) -> i32 {
        self.right - self.left
    }
    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }
}

impl From<RECT> for Rect {
    fn from(r: RECT) -> Self {
        Self { left: r.left, top: r.top, right: r.right, bottom: r.bottom }
    }
}

/// A connected display. `work` excludes taskbars and other docked bars.
#[derive(Clone, Debug, PartialEq)]
pub struct Display {
    /// `\\.\DISPLAY1`
    pub device: String,
    /// "Display 2 · DELL U2720Q · 3840 × 2160 (main)", translated.
    pub label: String,
    pub bounds: Rect,
    pub work: Rect,
    pub primary: bool,
    pub handle: isize,
}

/// Every connected display, ordered by number.
pub fn all() -> Vec<Display> {
    unsafe extern "system" fn add(monitor: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        let list = unsafe { &mut *(data.0 as *mut Vec<Display>) };
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if unsafe { GetMonitorInfoW(monitor, &mut info as *mut _ as *mut MONITORINFO) }.as_bool() {
            list.push(describe(&info, monitor));
        }
        true.into()
    }
    let mut list: Vec<Display> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(add), LPARAM(&mut list as *mut _ as isize));
    }
    list.sort_by_key(|d| number(&d.device));
    list
}

/// The display for a `Monitor` setting; falls back to the main display. `None` only with no display at all.
pub fn choose(setting: &str) -> Option<Display> {
    let all = all();
    let primary = all.iter().find(|d| d.primary).or(all.first())?.clone();
    if setting.is_empty() {
        return Some(primary);
    }
    if setting == FOLLOW_MOUSE {
        let mut p = POINT::default();
        if unsafe { GetCursorPos(&mut p) }.is_ok() {
            return Some(all.into_iter().find(|d| d.bounds.contains(p.x, p.y)).unwrap_or(primary));
        }
        return Some(primary);
    }
    Some(all.into_iter().find(|d| d.device.eq_ignore_ascii_case(setting)).unwrap_or(primary))
}

fn describe(info: &MONITORINFOEXW, monitor: HMONITOR) -> Display {
    let primary = info.monitorInfo.dwFlags & 1 != 0; // MONITORINFOF_PRIMARY
    let device = from_wide(&info.szDevice);
    let bounds: Rect = info.monitorInfo.rcMonitor.into();
    let mut label = l::f("Display {0}", &[&number(&device)]);
    if let Some(model) = model_name(&device) {
        label += &format!(" · {model}");
    }
    label += &format!(" · {} × {}", bounds.width(), bounds.height());
    if primary {
        label += &format!(" {}", l::t("(main)"));
    }
    Display { device, label, bounds, work: info.monitorInfo.rcWork.into(), primary, handle: monitor.0 as isize }
}

/// `\\.\DISPLAY2` -> 2
fn number(device: &str) -> u32 {
    let digits: String = device.chars().rev().take_while(char::is_ascii_digit).collect();
    digits.chars().rev().collect::<String>().parse().unwrap_or(0)
}

/// The monitor's model ("DELL U2720Q"), unless Windows only knows it as a generic monitor.
fn model_name(device: &str) -> Option<String> {
    let mut dd = DISPLAY_DEVICEW { cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32, ..Default::default() };
    let device = wide(device);
    if !unsafe { EnumDisplayDevicesW(PCWSTR(device.as_ptr()), 0, &mut dd, 0) }.as_bool() {
        return None;
    }
    let name = from_wide(&dd.DeviceString).trim().to_string();
    let lower = name.to_lowercase();
    (!name.is_empty() && !lower.contains("generic") && !lower.contains("pnp")).then_some(name)
}
