//! The right-click / tray menu in the island's colors (the C# app's restyled WPF ContextMenu): a rounded panel
//! with a 26-wide icon column, the shortcut on the right, submenus that open to the right, and a highlight under
//! the mouse. Drawn with the island's renderer in its own layered popup; it closes when clicked outside.

use super::colors::{self, Color};
use super::el::{Align, El, Font, Kind, Len, Run, Text, TextAlign};
use super::geom::{Point, Rect};
use super::render::{PaintState, Renderer};
use crate::rt::{self, guard};
use std::cell::RefCell;
use std::rc::Rc;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_DOWN, VK_ESCAPE, VK_LEFT, VK_RETURN, VK_RIGHT, VK_UP};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

/// One line of a menu.
#[derive(Clone)]
pub enum Item {
    Entry { label: String, glyph: Option<&'static str>, gesture: String, bold: bool, action: Option<Rc<dyn Fn()>>, submenu: Vec<Item> },
    Separator,
}

impl Item {
    pub fn new(label: impl Into<String>, glyph: Option<&'static str>, action: impl Fn() + 'static) -> Item {
        Item::Entry { label: label.into(), glyph, gesture: String::new(), bold: false, action: Some(Rc::new(action)), submenu: Vec::new() }
    }

    pub fn submenu(label: impl Into<String>, glyph: Option<&'static str>, items: Vec<Item>) -> Item {
        Item::Entry { label: label.into(), glyph, gesture: String::new(), bold: false, action: None, submenu: items }
    }

    pub fn bold(mut self) -> Item {
        if let Item::Entry { bold, .. } = &mut self {
            *bold = true;
        }
        self
    }

    pub fn gesture(mut self, text: &str) -> Item {
        if let Item::Entry { gesture, .. } = &mut self {
            *gesture = text.to_string();
        }
        self
    }
}

struct Palette {
    background: Color,
    foreground: Color,
    secondary: Color,
    border: Color,
    hover: Color,
}

fn palette() -> Palette {
    if colors::light() {
        Palette { background: Color::of("#F9F9F9"), foreground: Color::of("#1C1C1E"), secondary: Color::of("#6C6C70"), border: Color::of("#D1D1D6"), hover: Color::of("#E5E5EA") }
    } else {
        Palette { background: Color::of("#1C1C1E"), foreground: Color::WHITE, secondary: Color::of("#8E8E93"), border: Color::of("#3A3A3C"), hover: Color::of("#3A3A3C") }
    }
}

struct Popup {
    hwnd: HWND,
    items: Vec<Item>,
    renderer: Option<Renderer>,
    hovered: Option<usize>,
    /// Item rectangles (DIPs) after layout.
    rects: Vec<Rect>,
    scale: f64,
    size: (f64, f64),
    origin: POINT,
}

thread_local! {
    /// Open menus: the root, then its open submenu.
    static OPEN: RefCell<Vec<Popup>> = const { RefCell::new(Vec::new()) };
}

fn menu_text(text: &str, weight: u16, color: Color, align: TextAlign) -> El {
    El::new(Kind::Text(Text {
        runs: vec![Run { text: text.into(), size: 13.0, weight, color }],
        font: Font::Menu,
        tabular: false,
        wrap: false,
        align,
        trim: false,
        layout: Default::default(),
    }))
    .valign(Align::Center)
}

/// The menu as an element tree (rows are the children of the inner stack, in order).
fn view(items: &[Item], hovered: Option<usize>) -> El {
    let p = palette();
    let rows: Vec<El> = items
        .iter()
        .enumerate()
        .map(|(i, item)| match item {
            Item::Separator => El::empty().h(1.0).margin(8.0, 4.0, 8.0, 4.0).bg(p.border),
            Item::Entry { label, glyph, gesture, bold, submenu, .. } => {
                let mut cells = vec![(0, 1, menu_text(label, if *bold { 600 } else { 400 }, p.foreground, TextAlign::Left))];
                if let Some(g) = glyph {
                    cells.push((0, 0, super::glyph(g, 14.0, colors::primary()).halign(Align::Start)));
                }
                if !gesture.is_empty() {
                    cells.push((0, 2, menu_text(gesture, 400, p.secondary, TextAlign::Left).margin(24.0, 0.0, 0.0, 0.0)));
                }
                if !submenu.is_empty() {
                    let arrow = super::geom::Path::parse("M 0,0 L 4,4 L 0,8");
                    let shape = El::new(Kind::Shape(super::el::Shape { path: Rc::new(arrow), fill: None, stroke: Some(p.secondary), thickness: 1.4, round_caps: false, stretch: false }))
                        .size(4.0, 8.0)
                        .align(Align::End, Align::Center);
                    cells.push((0, 3, shape));
                }
                let grid = El::grid(vec![Len::Px(26.0), Len::Star(1.0), Len::Auto, Len::Px(16.0)], vec![], cells);
                let mut row = El::boxed(vec![grid]).padding(8.0, 6.0, 8.0, 6.0).radius(4.0);
                if hovered == Some(i) {
                    row = row.bg(p.hover);
                }
                row
            }
        })
        .collect();
    El::boxed(vec![El::vstack(rows)]).padding(5.0, 5.0, 5.0, 5.0).radius(8.0).bg(p.background).border(p.border, 1.0)
}

/// Natural size of the menu (DIPs), at least 210 wide per item plus the frame.
fn measure(items: &[Item]) -> (f64, f64) {
    let mut el = view(items, None);
    let size = el.measure(super::geom::Size::new(f64::INFINITY, f64::INFINITY));
    (size.w.max(210.0 + 10.0).ceil(), size.h.ceil())
}

fn dpi_scale(x: i32, y: i32) -> f64 {
    unsafe {
        let monitor = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        dx as f64 / 96.0
    }
}

fn work_area(x: i32, y: i32) -> RECT {
    unsafe {
        let monitor = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(monitor, &mut info);
        info.rcWork
    }
}

fn register() {
    unsafe {
        let instance = GetModuleHandleW(None).unwrap_or_default();
        let class = WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: instance.into(),
            lpszClassName: w!("DynamicIsland.Menu"),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassW(&class);
    }
}

/// Opens a menu with its top-left at the screen point (pixels), moved to stay on screen.
pub fn show(items: Vec<Item>, x: i32, y: i32) {
    close_all();
    open(items, x, y, false);
}

fn open(items: Vec<Item>, x: i32, y: i32, submenu: bool) -> Option<()> {
    register();
    let scale = dpi_scale(x, y);
    let (w, h) = measure(&items);
    let (pw, ph) = ((w * scale).ceil() as i32, (h * scale).ceil() as i32);
    let work = work_area(x, y);
    let mut left = x;
    let mut top = y;
    if left + pw > work.right {
        left = (work.right - pw).max(work.left);
    }
    if top + ph > work.bottom {
        top = (y - ph).max(work.top);
    }
    let ex = WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | if submenu { WS_EX_NOACTIVATE } else { Default::default() };
    let hwnd = unsafe {
        CreateWindowExW(ex, w!("DynamicIsland.Menu"), w!(""), WS_POPUP, left, top, pw, ph, None, None, Some(GetModuleHandleW(None).ok()?.into()), None).ok()?
    };
    OPEN.with(|o| {
        o.borrow_mut().push(Popup { hwnd, items, renderer: Renderer::new().ok(), hovered: None, rects: Vec::new(), scale, size: (w, h), origin: POINT { x: left, y: top } })
    });
    redraw(hwnd);
    unsafe {
        let _ = ShowWindow(hwnd, if submenu { SW_SHOWNOACTIVATE } else { SW_SHOW });
        if !submenu {
            let _ = SetForegroundWindow(hwnd);
            let _ = SetCapture(hwnd);
        }
    }
    Some(())
}

fn redraw(hwnd: HWND) {
    OPEN.with(|o| {
        let mut o = o.borrow_mut();
        let Some(popup) = o.iter_mut().find(|p| p.hwnd == hwnd) else { return };
        let mut el = view(&popup.items, popup.hovered);
        el.layout(Rect::new(0.0, 0.0, popup.size.0, popup.size.1));
        // Row rectangles: the children of the inner stack.
        popup.rects = el.children().first().map(|stack| stack.children().iter().map(|r| r.rect).collect()).unwrap_or_default();
        let (pw, ph) = ((popup.size.0 * popup.scale).ceil() as u32, (popup.size.1 * popup.scale).ceil() as u32);
        let state = PaintState::default();
        if let Some(r) = popup.renderer.as_mut() {
            let _ = r.present(hwnd, popup.origin, pw, ph, popup.scale, &state, &mut |p| p.paint(&el));
        }
    });
}

/// Closes every open menu.
pub fn close_all() {
    let popups: Vec<HWND> = OPEN.with(|o| o.borrow_mut().drain(..).map(|p| p.hwnd).collect());
    unsafe {
        let _ = ReleaseCapture();
        for hwnd in popups {
            let _ = DestroyWindow(hwnd);
        }
    }
}

fn close_submenus_after(level: usize) {
    let extra: Vec<HWND> = OPEN.with(|o| {
        let mut o = o.borrow_mut();
        if o.len() > level + 1 { o.drain(level + 1..).map(|p| p.hwnd).collect() } else { Vec::new() }
    });
    for hwnd in extra {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }
}

/// Which open menu (level) and row is under a screen point.
fn hit(screen: POINT) -> Option<(usize, Option<usize>)> {
    OPEN.with(|o| {
        let o = o.borrow();
        for (level, popup) in o.iter().enumerate().rev() {
            let (pw, ph) = (popup.size.0 * popup.scale, popup.size.1 * popup.scale);
            let (dx, dy) = ((screen.x - popup.origin.x) as f64, (screen.y - popup.origin.y) as f64);
            if dx < 0.0 || dy < 0.0 || dx >= pw || dy >= ph {
                continue;
            }
            let p = Point::new(dx / popup.scale, dy / popup.scale);
            let row = popup.rects.iter().position(|r| r.contains(p)).filter(|i| matches!(popup.items[*i], Item::Entry { .. }));
            return Some((level, row));
        }
        None
    })
}

fn set_hover(level: usize, row: Option<usize>) {
    let (hwnd, changed, submenu) = OPEN.with(|o| {
        let mut o = o.borrow_mut();
        let Some(popup) = o.get_mut(level) else { return (HWND::default(), false, None) };
        let changed = popup.hovered != row;
        popup.hovered = row;
        let submenu = row.and_then(|i| match &popup.items[i] {
            Item::Entry { submenu, .. } if !submenu.is_empty() => {
                let r = popup.rects[i];
                // To the right of the item (HorizontalOffset 2, VerticalOffset -5).
                let x = popup.origin.x + ((popup.size.0 + 2.0) * popup.scale) as i32;
                let y = popup.origin.y + ((r.y - 5.0) * popup.scale) as i32;
                Some((submenu.clone(), x, y))
            }
            _ => None,
        });
        (popup.hwnd, changed, submenu)
    });
    if !changed {
        return;
    }
    redraw(hwnd);
    close_submenus_after(level);
    if let Some((items, x, y)) = submenu {
        open(items, x, y, true);
    }
}

fn activate(level: usize, row: usize) {
    let item = OPEN.with(|o| o.borrow().get(level).map(|p| p.items[row].clone()));
    if let Some(Item::Entry { action: Some(action), .. }) = item {
        close_all();
        rt::defer(move || action());
    }
}

fn screen_point(hwnd: HWND, lparam: LPARAM) -> POINT {
    let mut p = POINT { x: (lparam.0 & 0xFFFF) as i16 as i32, y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32 };
    unsafe {
        let _ = windows::Win32::Graphics::Gdi::ClientToScreen(hwnd, &mut p);
    }
    p
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let handled = guard(|| -> Option<LRESULT> {
        match msg {
            WM_MOUSEMOVE => {
                let mut track = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                unsafe {
                    let _ = TrackMouseEvent(&mut track);
                }
                if let Some((level, row)) = hit(screen_point(hwnd, lparam)) {
                    // Over a submenu's parent row the submenu stays; elsewhere in a menu, rows follow the mouse.
                    if row.is_some() || level + 1 == OPEN.with(|o| o.borrow().len()) {
                        set_hover(level, row);
                    }
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONUP | WM_RBUTTONUP => {
                match hit(screen_point(hwnd, lparam)) {
                    Some((level, Some(row))) => activate(level, row),
                    Some(_) => {}
                    None => close_all(),
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONDOWN | WM_RBUTTONDOWN => {
                if hit(screen_point(hwnd, lparam)).is_none() {
                    close_all();
                }
                Some(LRESULT(0))
            }
            WM_KEYDOWN => {
                let key = wparam.0 as u16;
                let level = OPEN.with(|o| o.borrow().len()).saturating_sub(1);
                let (count, current, entries) = OPEN.with(|o| {
                    let o = o.borrow();
                    let p = &o[level];
                    (p.items.len(), p.hovered, p.items.iter().map(|i| matches!(i, Item::Entry { .. })).collect::<Vec<_>>())
                });
                let step = |from: Option<usize>, down: bool| -> Option<usize> {
                    let mut i = from.unwrap_or(if down { count - 1 } else { 0 });
                    for _ in 0..count {
                        i = if down { (i + 1) % count } else { (i + count - 1) % count };
                        if entries[i] {
                            return Some(i);
                        }
                    }
                    None
                };
                match key {
                    k if k == VK_ESCAPE.0 => {
                        if level == 0 {
                            close_all();
                        } else {
                            close_submenus_after(level - 1);
                        }
                    }
                    k if k == VK_LEFT.0 && level > 0 => close_submenus_after(level - 1),
                    k if k == VK_DOWN.0 => {
                        let next = step(current, true);
                        OPEN.with(|o| o.borrow_mut()[level].hovered = None);
                        set_hover(level, next);
                    }
                    k if k == VK_UP.0 => {
                        let next = step(current, false);
                        OPEN.with(|o| o.borrow_mut()[level].hovered = None);
                        set_hover(level, next);
                    }
                    k if k == VK_RETURN.0 || k == VK_RIGHT.0 => {
                        if let Some(row) = current {
                            let has_sub = OPEN.with(|o| matches!(&o.borrow()[level].items[row], Item::Entry { submenu, .. } if !submenu.is_empty()));
                            if has_sub {
                                OPEN.with(|o| {
                                    if let Some(sub) = o.borrow_mut().get_mut(level + 1) {
                                        sub.hovered = None;
                                    }
                                });
                                let sub_first = OPEN.with(|o| o.borrow().get(level + 1).and_then(|p| p.items.iter().position(|i| matches!(i, Item::Entry { .. }))));
                                if let Some(first) = sub_first {
                                    set_hover(level + 1, Some(first));
                                }
                            } else if k == VK_RETURN.0 {
                                activate(level, row);
                            }
                        }
                    }
                    _ => {}
                }
                Some(LRESULT(0))
            }
            WM_ACTIVATE if (wparam.0 & 0xFFFF) == 0 => {
                // Clicked elsewhere, or another window came to the front.
                rt::defer(close_all);
                Some(LRESULT(0))
            }
            WM_CAPTURECHANGED => {
                let still_open = OPEN.with(|o| o.borrow().first().is_some_and(|p| p.hwnd == hwnd));
                if still_open && lparam.0 != hwnd.0 as isize {
                    // Keep receiving clicks outside the menu so they close it.
                    let other_menu = OPEN.with(|o| o.borrow().iter().any(|p| p.hwnd.0 as isize == lparam.0));
                    if !other_menu {
                        rt::defer(close_all);
                    }
                }
                Some(LRESULT(0))
            }
            WM_MOUSEACTIVATE => {
                let root = OPEN.with(|o| o.borrow().first().is_some_and(|p| p.hwnd == hwnd));
                Some(LRESULT(if root { MA_ACTIVATE } else { MA_NOACTIVATE } as isize))
            }
            _ => None,
        }
    })
    .flatten();
    handled.unwrap_or_else(|| unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) })
}
