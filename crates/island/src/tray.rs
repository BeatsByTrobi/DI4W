//! The notification-area (tray) icon: one click brings a hidden island back, a double-click opens the settings,
//! right-click opens the menu. It also receives Windows' broadcasts that only top-level windows get (the light/dark
//! switch) and comes back when Explorer restarts.

use crate::rt::guard;
use crate::ui::tray_icons;
use std::cell::RefCell;
use std::rc::Rc;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

const WM_TRAY: u32 = WM_APP + 10;

/// What the tray reports.
pub enum Event {
    Click,
    DoubleClick,
    /// Right-click, at this screen point.
    Menu(i32, i32),
    /// Windows switched between light and dark.
    ThemeChanged,
}

struct Tray {
    hwnd: HWND,
    normal: HICON,
    paused: HICON,
    is_paused: bool,
    tip: String,
    on_event: Rc<dyn Fn(Event)>,
    taskbar_created: u32,
}

thread_local! {
    static TRAY: RefCell<Option<Tray>> = const { RefCell::new(None) };
}

fn data(t: &Tray) -> NOTIFYICONDATAW {
    let mut d = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: t.hwnd,
        uID: 1,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP,
        uCallbackMessage: WM_TRAY,
        hIcon: if t.is_paused { t.paused } else { t.normal },
        ..Default::default()
    };
    for (i, c) in t.tip.encode_utf16().take(127).enumerate() {
        d.szTip[i] = c;
    }
    d
}

fn add(t: &Tray) {
    let mut d = data(t);
    unsafe {
        let _ = Shell_NotifyIconW(NIM_ADD, &d);
        d.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        let _ = Shell_NotifyIconW(NIM_SETVERSION, &d);
    }
}

/// Creates the icon (`tip`: its tooltip).
pub fn create(tip: &str, on_event: impl Fn(Event) + 'static) {
    unsafe {
        let instance = GetModuleHandleW(None).unwrap_or_default();
        let class = WNDCLASSW { lpfnWndProc: Some(proc), hInstance: instance.into(), lpszClassName: w!("DynamicIsland.Tray"), ..Default::default() };
        RegisterClassW(&class);
        let hwnd = CreateWindowExW(WS_EX_TOOLWINDOW, w!("DynamicIsland.Tray"), w!("Dynamic Island Tray"), WS_POPUP, 0, 0, 0, 0, None, None, Some(instance.into()), None).unwrap_or_default();
        let tray = Tray {
            hwnd,
            normal: tray_icons::normal(),
            paused: tray_icons::paused(),
            is_paused: false,
            tip: tip.into(),
            on_event: Rc::new(on_event),
            taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
        };
        add(&tray);
        TRAY.with(|t| *t.borrow_mut() = Some(tray));
    }
}

/// Normal or paused (faded, with an orange pause badge), with a matching tooltip.
pub fn set_paused(paused: bool, tip: &str) {
    let d = TRAY.with(|t| {
        let mut t = t.borrow_mut();
        let tray = t.as_mut()?;
        tray.is_paused = paused;
        tray.tip = tip.into();
        Some(data(tray))
    });
    if let Some(d) = d {
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &d);
        }
    }
}

pub fn remove() {
    // Out of the cell first: deleting the icon and the window sends messages to `proc`, which reads it.
    if let Some(tray) = TRAY.with(|t| t.borrow_mut().take()) {
        let d = data(&tray);
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &d);
            let _ = DestroyWindow(tray.hwnd);
        }
    }
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let handled = guard(|| -> Option<LRESULT> {
        let (callback, taskbar_created) = TRAY.with(|t| t.try_borrow().ok()?.as_ref().map(|t| (t.on_event.clone(), t.taskbar_created)))?;
        if msg == taskbar_created {
            // Explorer restarted: the icon is gone, add it again.
            TRAY.with(|t| {
                if let Some(tray) = t.borrow().as_ref() {
                    add(tray);
                }
            });
            return Some(LRESULT(0));
        }
        match msg {
            WM_TRAY => {
                let event = (lparam.0 & 0xFFFF) as u32;
                let (x, y) = ((wparam.0 & 0xFFFF) as i16 as i32, ((wparam.0 >> 16) & 0xFFFF) as i16 as i32);
                match event {
                    WM_LBUTTONUP => callback(Event::Click),
                    WM_LBUTTONDBLCLK => callback(Event::DoubleClick),
                    WM_CONTEXTMENU => callback(Event::Menu(x, y)),
                    _ => {}
                }
                Some(LRESULT(0))
            }
            WM_SETTINGCHANGE => {
                if lparam.0 != 0 {
                    let text = unsafe { windows::core::PCWSTR(lparam.0 as *const u16).to_string() }.unwrap_or_default();
                    if text == "ImmersiveColorSet" {
                        callback(Event::ThemeChanged);
                    }
                }
                None
            }
            _ => None,
        }
    })
    .flatten();
    handled.unwrap_or_else(|| unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) })
}
