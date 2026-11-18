//! The window's Windows side: Windows' own title bar is dropped (the content reaches the top edge, where the
//! macOS-style traffic lights sit) while the resizable border, shadow and snapping stay; Mica behind the
//! content on Windows 11; dark or light frame; and its start size and place.

use std::cell::RefCell;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWINDOWATTRIBUTE, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint, ScreenToClient};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, GetDpiForWindow, GetSystemMetricsForDpi, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    HTCAPTION, HTCLIENT, HTTOP, HTTOPLEFT, HTTOPRIGHT, IsZoomed, NCCALCSIZE_PARAMS, SM_CXPADDEDBORDER, SM_CYFRAME,
    SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos, WA_INACTIVE, WM_ACTIVATE, WM_NCCALCSIZE,
    WM_NCHITTEST, WM_SETTINGCHANGE,
};

/// The title bar's height and where the traffic lights are in it, in logical pixels (see app.slint).
const TITLE_BAR_HEIGHT: f64 = 48.0;
const LIGHTS: (f64, f64, f64, f64) = (18.0, 14.0, 18.0 + 56.0, 34.0);
/// Near-misses around the dots still hit a button instead of dragging the window.
const LIGHTS_SLACK: f64 = 4.0;

type OnActive = Box<dyn Fn(bool)>;
type OnTheme = Box<dyn Fn()>;

thread_local! {
    static ON_ACTIVE: RefCell<Option<OnActive>> = const { RefCell::new(None) };
    static ON_THEME: RefCell<Option<OnTheme>> = const { RefCell::new(None) };
}

pub fn hwnd(window: &slint::Window) -> Option<HWND> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let handle = window.window_handle();
    match handle.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(HWND(h.hwnd.get() as *mut _)),
        _ => None,
    }
}

/// Windows 11 22H2 or later (Mica as a system backdrop).
pub fn supports_mica() -> bool {
    windows_build() >= 22621 && std::env::var_os("DI4W_NO_MICA").is_none()
}

fn windows_build() -> u32 {
    crate::system::windows_build().unwrap_or(0)
}

/// Takes over the title bar, sets Mica and the frame colors. `on_active` hears when the window is activated
/// or goes to the background; `on_theme` when Windows switches between light and dark.
pub fn customize(hwnd: HWND, dark: bool, mica: bool, on_active: impl Fn(bool) + 'static, on_theme: impl Fn() + 'static) {
    ON_ACTIVE.with(|c| *c.borrow_mut() = Some(Box::new(on_active)));
    ON_THEME.with(|c| *c.borrow_mut() = Some(Box::new(on_theme)));
    unsafe {
        let _ = SetWindowSubclass(hwnd, Some(subclass), 0x4449_3457, 0);
        set_dark_frame(hwnd, dark);
        if mica {
            let backdrop: i32 = 2; // DWMSBT_MAINWINDOW (Mica)
            let _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(38), &backdrop as *const _ as *const _, 4);
        }
        // Recalculate the frame now that WM_NCCALCSIZE answers differently.
        let _ = SetWindowPos(hwnd, None, 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER);
    }
}

pub fn set_dark_frame(hwnd: HWND, dark: bool) {
    let value: i32 = dark.into();
    unsafe {
        let _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(20), &value as *const _ as *const _, 4); // DWMWA_USE_IMMERSIVE_DARK_MODE
    }
}

/// The resize border's thickness (also how far a maximized window reaches past the screen).
fn frame_thickness(hwnd: HWND) -> i32 {
    unsafe {
        let dpi = GetDpiForWindow(hwnd);
        GetSystemMetricsForDpi(SM_CYFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi)
    }
}

unsafe extern "system" fn subclass(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, _data: usize) -> LRESULT {
    match msg {
        // The client area reaches the top edge; left, right and bottom keep Windows' resize border.
        WM_NCCALCSIZE if wparam.0 != 0 => {
            let params = unsafe { &mut *(lparam.0 as *mut NCCALCSIZE_PARAMS) };
            let top = params.rgrc[0].top;
            let result = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
            params.rgrc[0].top = top + if unsafe { IsZoomed(hwnd) }.as_bool() { frame_thickness(hwnd) } else { 0 };
            result
        }
        WM_NCHITTEST => {
            let hit = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
            if hit.0 as u32 != HTCLIENT {
                return hit; // the left, right and bottom borders
            }
            let mut point = POINT { x: (lparam.0 & 0xFFFF) as i16 as i32, y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32 };
            unsafe {
                let _ = ScreenToClient(hwnd, &mut point);
            }
            let scale = unsafe { GetDpiForWindow(hwnd) } as f64 / 96.0;
            let (x, y) = (point.x as f64 / scale, point.y as f64 / scale);
            let maximized = unsafe { IsZoomed(hwnd) }.as_bool();
            let border = frame_thickness(hwnd);
            if !maximized && point.y < border {
                let mut rect = RECT::default();
                unsafe {
                    let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut rect);
                }
                return LRESULT(if point.x < border * 2 {
                    HTTOPLEFT
                } else if point.x > rect.right - border * 2 {
                    HTTOPRIGHT
                } else {
                    HTTOP
                } as isize);
            }
            let on_lights = x >= LIGHTS.0 - LIGHTS_SLACK
                && x <= LIGHTS.2 + LIGHTS_SLACK
                && y >= LIGHTS.1 - LIGHTS_SLACK
                && y <= LIGHTS.3 + LIGHTS_SLACK;
            if y < TITLE_BAR_HEIGHT && !on_lights {
                return LRESULT(HTCAPTION as isize);
            }
            hit
        }
        WM_ACTIVATE => {
            let active = (wparam.0 & 0xFFFF) as u32 != WA_INACTIVE;
            let _ = slint::invoke_from_event_loop(move || ON_ACTIVE.with(|c| c.borrow().as_ref().map(|f| f(active))).unwrap_or(()));
            unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
        }
        WM_SETTINGCHANGE => {
            // "ImmersiveColorSet": Windows switched between light and dark.
            let name = lparam.0 as *const u16;
            if !name.is_null() {
                let text = unsafe { windows::core::PCWSTR(name).to_string() }.unwrap_or_default();
                if text == "ImmersiveColorSet" {
                    let _ = slint::invoke_from_event_loop(|| ON_THEME.with(|c| c.borrow().as_ref().map(|f| f())).unwrap_or(()));
                }
            }
            unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
        }
        _ => unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) },
    }
}

/// 1000 × 720 (as far as it fits), centered on the main display's work area.
pub fn place_on_main_display(window: &slint::Window) {
    unsafe {
        let monitor = MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return;
        }
        let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
        let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
        let scale = dpi_x as f32 / 96.0;
        let work = info.rcWork;
        let width = ((1000.0 * scale) as i32).min(work.right - work.left);
        let height = ((720.0 * scale) as i32).min(work.bottom - work.top);
        window.set_size(slint::PhysicalSize::new(width as u32, height as u32));
        window.set_position(slint::PhysicalPosition::new(
            work.left + (work.right - work.left - width) / 2,
            work.top + (work.bottom - work.top - height) / 2,
        ));
    }
}
