//! The island window: the black pill at the top of the screen (the C# app's `IslandWindow`).
//!
//! Topmost, no taskbar entry, never takes focus, and click-through everywhere except what it draws (a layered
//! window: fully transparent pixels let the mouse through). It sits at the top-center of the chosen display, below
//! a taskbar docked at the top, and hides while a fullscreen app runs (unless game mode keeps it).
//!
//! What it shows follows [`Manager`]: an alert, an open panel (the timer picker), the expanded activity, the
//! compact activity (two at once: the second buds off into a detached bubble), or the resting island. The shape
//! follows springs (size with a little bounce, radius without), content fades in with a blur, a press squeezes it.

use crate::accessibility;
use crate::animation::{Ease, Spring, Tween};
use crate::model::{Activity, Alert, Manager, power};
use crate::rt::{self, Timer};
use crate::ui::colors::{self, Color};
use crate::ui::el::{Align, Cursor, El, Kind};
use crate::ui::geom::{Path, Point, Rect, Size};
use crate::ui::render::{PaintState, Renderer, scale_around};
use crate::ui::{self, shape};
use di4w_core::{AppSettings, l};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

const CANVAS_WIDTH: f64 = 600.0;
const CANVAS_HEIGHT: f64 = 260.0;
// Apple's measurements in points (iPhone 15/16 Pro, HIG "Live Activities"): the resting island is 126 × 37.33,
// compact activities add about 52 on each side, expanded ones are 371 wide and 84–160 tall.
const COMPACT_HEIGHT: f64 = 37.0;
const IDLE_WIDTH: f64 = 126.0;
const IDLE_WITH_CONTENT_WIDTH: f64 = 172.0; // room for the weather / Do Not Disturb moon beside the camera
const BUBBLE_SIZE: f64 = 37.0;
const BUBBLE_GAP: f64 = 7.0;
const MAX_RADIUS: f64 = 44.0;
/// The neck is gone before the bubble settles at its gap.
const NECK_REACH: f64 = BUBBLE_GAP - 2.0;

const NEUTRAL_KEYLINE: Color = Color::argb(0x24, 0xFF, 0xFF, 0xFF);
const LIGHT_KEYLINE: Color = Color::argb(0x22, 0x00, 0x00, 0x00);

const HIDE_HOTKEY: i32 = 1;
const EXPAND_HOTKEY: i32 = 2;

/// An animation frame is due (posted by the frame clock, in step with the display).
const WM_FRAME: u32 = WM_APP + 2;
const WM_MOUSELEAVE: u32 = 0x02A3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Idle,
    Compact,
    Expanded,
    Panel,
    Alert,
}

/// Identity of what's shown; a new one re-runs the reveal animation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Key {
    None,
    Idle,
    Alert(usize),
    Panel(u64),
    Expanded(usize),
    Compact(usize, bool),
}

fn addr(a: &Rc<dyn Activity>) -> usize {
    Rc::as_ptr(a) as *const () as usize
}

/// Content shown in the island's panel slot (the timer picker opened from the idle island).
#[derive(Clone)]
pub struct Panel {
    pub id: u64,
    pub view: Rc<dyn Fn() -> El>,
    pub size: Size,
}

/// A small status dot right of the pill (privacy indicators).
#[derive(Clone)]
pub struct Dot {
    pub color: Color,
    pub visible: bool,
    pub tooltip: Option<String>,
}

#[derive(Clone)]
enum Content {
    None,
    Idle,
    Compact(Rc<dyn Activity>, bool),
    Expanded(Rc<dyn Activity>),
    Panel(Panel),
    Alert(Rc<Alert>),
}

/// One of the four layers (idle, compact, expanded/panel, alert) with its fade, grow and blur.
struct Layer {
    content: Content,
    size: Size,
    opacity: Tween,
    scale: Tween,
    blur: Tween,
    visible: bool,
    hit: bool,
}

impl Layer {
    fn new() -> Layer {
        Layer { content: Content::None, size: Size::default(), opacity: Tween::fixed(0.0), scale: Tween::fixed(1.0), blur: Tween::fixed(0.0), visible: false, hit: false }
    }
}

const IDLE: usize = 0;
const COMPACT: usize = 1;
const EXPANDED: usize = 2;
const ALERT: usize = 3;

struct State {
    mode: Mode,
    shown_key: Key,
    expanded: bool,
    hovering: bool,
    panel: Option<Panel>,
    secondary: Option<Rc<dyn Activity>>,
    /// What the bubble shows (kept while it shrinks back in).
    bubble_content: Option<Rc<dyn Activity>>,
    bubble_width: f64,
    width: Spring,
    height: Spring,
    radius: Spring,
    bubble: Spring,
    press: Spring,
    layers: [Layer; 4],
    keyline: Option<Color>,
    hover_target: Option<Rc<dyn Activity>>,
    fullscreen_ticks: u32,
    visible: bool,
    last_frame: Option<Instant>,
    // input
    pressed_button: Option<u64>,
    press_tweens: HashMap<u64, Tween>,
    hovered_button: Option<u64>,
    tracking: bool,
    cursor: Cursor,
    // the last frame's laid-out views, for hit testing (canvas DIPs)
    hit_layers: Vec<El>,
    bubble_el: Option<El>,
    pill_outline: Path,
    pill_rect: Rect,
    bubble_rect: Rect,
    bubble_hit: bool,
    indicator_rects: Vec<(Rect, Option<String>)>,
    // timers
    collapse: Option<Timer>,
    keyboard_collapse: Option<Timer>,
    housekeeping: Option<Timer>,
    announced: usize,
    refresh_queued: bool,
    redraw_queued: bool,
    /// Content asked for continuous frames (a pulsing icon).
    content_animating: bool,
    clock_start: Instant,
}

/// The island window.
pub struct Island {
    hwnd: HWND,
    manager: Rc<Manager>,
    settings: Rc<RefCell<AppSettings>>,
    renderer: RefCell<Option<Renderer>>,
    st: RefCell<State>,
    idle: RefCell<(Option<Rc<dyn Fn() -> El>>, Option<Rc<dyn Fn() -> El>>)>,
    dots: RefCell<Vec<Dot>>,
    /// Content shown when the idle island is clicked (the timer picker).
    pub idle_panel: RefCell<Option<Rc<dyn Fn() -> Panel>>>,
    /// Opens the right-click menu at a screen point (pixels).
    pub context_menu: RefCell<Option<Rc<dyn Fn(i32, i32)>>>,
    /// The global hide/show shortcut was pressed.
    pub hotkey_pressed: RefCell<Option<Rc<dyn Fn()>>>,
    /// Shown again after being hidden.
    pub on_shown: RefCell<Option<Rc<dyn Fn()>>>,
    /// Closing the window (installer, Windows shutting down) quits the app.
    pub on_close: RefCell<Option<Rc<dyn Fn()>>>,
    /// Every window message (msg, wParam, lParam), for add-ons that listen to broadcasts such as device changes or
    /// clipboard updates.
    hooks: RefCell<Vec<(u64, Rc<dyn Fn(u32, WPARAM, LPARAM)>)>>,
    next_hook: Cell<u64>,
    /// Set by the game add-on so a fullscreen game doesn't hide the island.
    pub keep_visible_in_fullscreen: Cell<bool>,
    /// Hidden by the user (App.Paused): housekeeping doesn't show it again.
    pub paused: Cell<bool>,
}

thread_local! {
    static ISLAND: RefCell<Option<Rc<Island>>> = const { RefCell::new(None) };
}

/// The island (UI thread), once created.
pub fn get() -> Option<Rc<Island>> {
    ISLAND.with(|i| i.borrow().clone())
}

// ───────────────────────────── frame clock ─────────────────────────────

static FRAME_WINDOW: AtomicIsize = AtomicIsize::new(0);
static ANIMATING: AtomicBool = AtomicBool::new(false);
static FRAME_PENDING: AtomicBool = AtomicBool::new(false);
static CLOCK: (Mutex<()>, Condvar) = (Mutex::new(()), Condvar::new());

/// Posts a frame message once per display refresh while something animates (WPF's CompositionTarget.Rendering).
fn start_frame_clock() {
    std::thread::spawn(|| loop {
        if !ANIMATING.load(Ordering::SeqCst) {
            let guard = CLOCK.0.lock().unwrap_or_else(|e| e.into_inner());
            let _ = CLOCK.1.wait_timeout(guard, Duration::from_millis(500));
            continue;
        }
        if unsafe { DwmFlush() }.is_err() {
            std::thread::sleep(Duration::from_millis(16));
        }
        if !FRAME_PENDING.swap(true, Ordering::SeqCst) {
            let hwnd = HWND(FRAME_WINDOW.load(Ordering::SeqCst) as *mut _);
            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_FRAME, WPARAM(0), LPARAM(0));
            }
        }
    });
}

fn set_animating(on: bool) {
    if ANIMATING.swap(on, Ordering::SeqCst) != on && on {
        CLOCK.1.notify_all();
    }
}

impl Island {
    /// Creates the window (hidden until [`Island::show`]).
    pub fn create(manager: Rc<Manager>, settings: Rc<RefCell<AppSettings>>) -> Rc<Island> {
        let hwnd = unsafe {
            let instance = GetModuleHandleW(None).unwrap_or_default();
            let class = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: instance.into(),
                lpszClassName: w!("DynamicIsland.Island"),
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                ..Default::default()
            };
            RegisterClassW(&class);
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                w!("DynamicIsland.Island"),
                w!("Dynamic Island"),
                WS_POPUP,
                0,
                0,
                600,
                260,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .unwrap_or_default()
        };
        FRAME_WINDOW.store(hwnd.0 as isize, Ordering::SeqCst);
        start_frame_clock();

        let island = Rc::new(Island {
            hwnd,
            manager: manager.clone(),
            settings,
            renderer: RefCell::new(Renderer::new().map_err(|e| di4w_core::log::error_with("Direct2D unavailable", e)).ok()),
            st: RefCell::new(State {
                mode: Mode::Idle,
                shown_key: Key::None,
                expanded: false,
                hovering: false,
                panel: None,
                secondary: None,
                bubble_content: None,
                bubble_width: BUBBLE_SIZE,
                // Shape springs, in SwiftUI terms: the size follows a quick spring with a little bounce (Apple's
                // island overshoots slightly and settles), the radius a stiffer one without bounce so the corners
                // never wobble.
                width: Spring::from_response(IDLE_WIDTH, 0.42, 0.72),
                height: Spring::from_response(COMPACT_HEIGHT, 0.42, 0.72),
                radius: Spring::from_response(COMPACT_HEIGHT / 2.0, 0.34, 1.0),
                bubble: Spring::from_response(0.0, 0.45, 0.62),
                // Pressing the island squeezes it a little, like a tap on the iPhone; letting go springs it back.
                press: Spring::from_response(0.0, 0.22, 0.6),
                layers: [Layer::new(), Layer::new(), Layer::new(), Layer::new()],
                keyline: None,
                hover_target: None,
                fullscreen_ticks: 0,
                visible: false,
                last_frame: None,
                pressed_button: None,
                press_tweens: HashMap::new(),
                hovered_button: None,
                tracking: false,
                cursor: Cursor::Inherit,
                hit_layers: Vec::new(),
                bubble_el: None,
                pill_outline: Path::default(),
                pill_rect: Rect::default(),
                bubble_rect: Rect::default(),
                bubble_hit: false,
                indicator_rects: Vec::new(),
                collapse: None,
                keyboard_collapse: None,
                housekeeping: None,
                announced: 0,
                refresh_queued: false,
                redraw_queued: false,
                content_animating: false,
                clock_start: Instant::now(),
            }),
            idle: RefCell::new((None, None)),
            dots: RefCell::new(Vec::new()),
            idle_panel: RefCell::new(None),
            context_menu: RefCell::new(None),
            hotkey_pressed: RefCell::new(None),
            on_shown: RefCell::new(None),
            on_close: RefCell::new(None),
            hooks: RefCell::new(Vec::new()),
            next_hook: Cell::new(1),
            keep_visible_in_fullscreen: Cell::new(false),
            paused: Cell::new(false),
        });
        ISLAND.with(|i| *i.borrow_mut() = Some(island.clone()));

        let weak = Rc::downgrade(&island);
        manager.set_on_changed(move || {
            if let Some(i) = weak.upgrade() {
                i.schedule_refresh();
            }
        });

        unsafe {
            // Re-assert topmost the moment any other window comes to the front, instead of up to a second later.
            SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(foreground_changed), 0, 0, WINEVENT_OUTOFCONTEXT);
        }

        // Battery saver: drop the soft shadow and the blur, and check less often.
        let weak = Rc::downgrade(&island);
        power::on_change(move || {
            if let Some(i) = weak.upgrade() {
                i.start_housekeeping();
                i.request_redraw();
            }
        });
        island.start_housekeeping();
        island.refresh();
        island
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    fn start_housekeeping(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        let interval = power::interval(Duration::from_secs(1), 2.0);
        self.st.borrow_mut().housekeeping = Some(Timer::every(interval, move || {
            if let Some(i) = weak.upgrade() {
                i.housekeeping();
            }
        }));
    }

    // ───────────────────────────── public API ─────────────────────────────

    pub fn is_visible(&self) -> bool {
        self.st.borrow().visible
    }

    pub fn show(self: &Rc<Self>) {
        self.set_visible(true);
    }

    pub fn hide(self: &Rc<Self>) {
        self.set_visible(false);
    }

    /// Hidden (by the user, or for a fullscreen app): alerts go to Recent instead of piling up, and the island comes
    /// back collapsed; the mouse can't still be over it, so a hover that never got its "leave" is reset.
    fn set_visible(self: &Rc<Self>, visible: bool) {
        {
            let mut st = self.st.borrow_mut();
            if st.visible == visible {
                return;
            }
            st.visible = visible;
            if !visible {
                st.hovering = false;
                st.expanded = false;
                st.collapse = None;
                st.keyboard_collapse = None;
                st.panel = None;
                st.hovered_button = None;
                st.pressed_button = None;
            }
        }
        unsafe {
            let _ = ShowWindow(self.hwnd, if visible { SW_SHOWNOACTIVATE } else { SW_HIDE });
        }
        if visible {
            self.position();
        }
        self.manager.set_suppressed(!visible);
        self.refresh();
        if visible {
            let f = self.on_shown.borrow().clone();
            if let Some(f) = f {
                rt::defer(move || f());
            }
        }
    }

    pub fn open_panel(self: &Rc<Self>, panel: Panel) {
        self.st.borrow_mut().panel = Some(panel);
        self.refresh();
    }

    pub fn close_panel(self: &Rc<Self>) {
        self.st.borrow_mut().panel = None;
        self.refresh();
    }

    /// Small status items shown in the empty pill (left: Do Not Disturb moon, right: weather).
    pub fn set_idle_content(self: &Rc<Self>, leading: Option<Rc<dyn Fn() -> El>>, trailing: Option<Rc<dyn Fn() -> El>>) {
        {
            let idle = self.idle.borrow();
            let same = |a: &Option<Rc<dyn Fn() -> El>>, b: &Option<Rc<dyn Fn() -> El>>| match (a, b) {
                (None, None) => true,
                (Some(a), Some(b)) => std::ptr::addr_eq(Rc::as_ptr(a), Rc::as_ptr(b)),
                _ => false,
            };
            if same(&idle.0, &leading) && same(&idle.1, &trailing) {
                return;
            }
        }
        *self.idle.borrow_mut() = (leading, trailing);
        {
            let mut st = self.st.borrow_mut();
            if st.mode == Mode::Idle {
                st.shown_key = Key::None; // re-run the reveal so new content fades in
            }
        }
        self.refresh();
    }

    fn has_idle_content(&self) -> bool {
        let idle = self.idle.borrow();
        idle.0.is_some() || idle.1.is_some()
    }

    /// The privacy dots right of the pill.
    pub fn set_dots(&self, dots: Vec<Dot>) {
        *self.dots.borrow_mut() = dots;
        self.request_redraw();
    }

    pub fn collapse(self: &Rc<Self>) {
        {
            let mut st = self.st.borrow_mut();
            st.expanded = false;
            st.panel = None;
        }
        self.refresh();
    }

    /// Adds a listener for every window message; returns an id for [`Island::remove_hook`].
    pub fn add_hook(&self, f: impl Fn(u32, WPARAM, LPARAM) + 'static) -> u64 {
        let id = self.next_hook.get();
        self.next_hook.set(id + 1);
        self.hooks.borrow_mut().push((id, Rc::new(f)));
        id
    }

    pub fn remove_hook(&self, id: u64) {
        self.hooks.borrow_mut().retain(|(i, _)| *i != id);
    }

    /// Registers (or replaces) the global hide/show shortcut. Empty or invalid text just removes it.
    pub fn register_hide_hotkey(&self, shortcut: &str) {
        self.register_hotkey(HIDE_HOTKEY, shortcut, "Hide");
    }

    /// The shortcut that opens/closes the expanded view, so the island can be used without a mouse.
    pub fn register_expand_hotkey(&self, shortcut: &str) {
        self.register_hotkey(EXPAND_HOTKEY, shortcut, "Expand");
    }

    fn register_hotkey(&self, id: i32, shortcut: &str, purpose: &str) {
        unsafe {
            let _ = UnregisterHotKey(Some(self.hwnd), id);
            let Some((modifiers, key)) = di4w_core::hotkey::parse(shortcut) else { return };
            match RegisterHotKey(Some(self.hwnd), id, HOT_KEY_MODIFIERS(modifiers) | MOD_NOREPEAT, key) {
                Ok(()) => di4w_core::log::info(format!("{purpose} shortcut: {shortcut}")),
                Err(e) => di4w_core::log::error(format!("Hotkey registration failed for {shortcut} ({e}); another app probably uses it")),
            }
        }
    }

    /// Opens the current activity (or the timer picker when idle), or closes it again.
    pub fn toggle_expanded(self: &Rc<Self>) {
        if !self.is_visible() {
            return; // hidden (paused, or a fullscreen app): it would open unseen and pop up open later
        }
        let open = {
            let st = self.st.borrow();
            st.expanded || st.panel.is_some()
        };
        if open {
            self.collapse();
            return;
        }
        if !self.manager.active().is_empty() {
            self.st.borrow_mut().expanded = true;
            self.refresh();
        } else {
            let factory = self.idle_panel.borrow().clone();
            if let Some(f) = factory {
                self.open_panel(f());
            }
        }
        // Without a mouse over it, it would stay open forever; close after a while like a hover-out would.
        let weak = Rc::downgrade(self);
        let mut st = self.st.borrow_mut();
        st.collapse = None;
        st.keyboard_collapse = Some(Timer::once(Duration::from_secs(8), move || {
            if let Some(i) = weak.upgrade() {
                i.st.borrow_mut().keyboard_collapse = None;
                if !i.st.borrow().hovering {
                    i.collapse();
                }
            }
        }));
    }

    /// The size setting or display changed: new window size and position.
    pub fn apply_scale(self: &Rc<Self>) {
        self.position();
        self.request_redraw();
    }

    // ───────────────────────────── state → shape ─────────────────────────────

    fn schedule_refresh(self: &Rc<Self>) {
        {
            let mut st = self.st.borrow_mut();
            if st.refresh_queued {
                return;
            }
            st.refresh_queued = true;
        }
        let weak = Rc::downgrade(self);
        rt::defer(move || {
            if let Some(i) = weak.upgrade() {
                i.st.borrow_mut().refresh_queued = false;
                i.refresh();
            }
        });
    }

    /// Works out what to show and starts the springs and fades towards it.
    pub fn refresh(self: &Rc<Self>) {
        let alert = self.manager.current_alert();
        let active = self.manager.active();
        let primary = active.first().cloned();
        let has_idle = self.has_idle_content();
        let now = self.now();
        {
            let mut st = self.st.borrow_mut();
            st.secondary = active.get(1).cloned();
            if primary.is_none() {
                st.expanded = false;
            }

            let (mode, target, key);
            if let Some(a) = &alert {
                mode = Mode::Alert;
                target = a.size;
                key = Key::Alert(Rc::as_ptr(a) as usize);
                st.layers[ALERT].content = Content::Alert(a.clone());
            } else if let Some(p) = st.panel.clone() {
                mode = Mode::Panel;
                target = p.size;
                key = Key::Panel(p.id);
                st.layers[EXPANDED].content = Content::Panel(p);
            } else if let (true, Some(p)) = (st.expanded, &primary) {
                mode = Mode::Expanded;
                target = p.expanded_size();
                key = Key::Expanded(addr(p));
                st.layers[EXPANDED].content = Content::Expanded(p.clone());
            } else if let Some(p) = &primary {
                mode = Mode::Compact;
                // Two at once, as on the iPhone: the first shows its minimal view attached to the island's leading
                // side, the second its minimal view in a detached bubble. One alone gets the full compact view.
                let split = st.secondary.is_some();
                target = if split { Size::new(IDLE_WIDTH + p.minimal_width(), COMPACT_HEIGHT) } else { Size::new(p.compact_width(), COMPACT_HEIGHT) };
                key = Key::Compact(addr(p), split);
                st.layers[COMPACT].content = Content::Compact(p.clone(), split);
            } else {
                mode = Mode::Idle;
                target = Size::new(if has_idle { IDLE_WITH_CONTENT_WIDTH } else { IDLE_WIDTH }, COMPACT_HEIGHT);
                key = Key::Idle;
                st.layers[IDLE].content = Content::Idle;
            }

            // Bubble only next to the compact pill. While it shrinks back in, it keeps what it showed.
            let show_bubble = mode == Mode::Compact && st.secondary.is_some();
            if show_bubble {
                let secondary = st.secondary.clone().unwrap();
                st.bubble_width = secondary.minimal_width();
                st.bubble_content = Some(secondary);
            }
            st.bubble.target = if show_bubble { 1.0 } else { 0.0 };
            st.bubble_hit = show_bubble;

            st.width.target = target.w;
            st.height.target = target.h;
            st.radius.target = (target.h / 2.0).min(MAX_RADIUS);
            st.keyline = match mode {
                Mode::Alert => alert.as_ref().and_then(|a| a.keyline),
                Mode::Compact | Mode::Expanded => primary.as_ref().and_then(|p| p.keyline()),
                _ => None,
            };

            if mode != st.mode || key != st.shown_key {
                Self::switch_layers(&mut st, mode, target, has_idle, now);
                st.mode = mode;
                st.shown_key = key;
            }
        }
        self.update_hover_target();
        self.update_accessible_name();
        self.kick();
    }

    fn switch_layers(st: &mut State, mode: Mode, size: Size, has_idle: bool, now: f64) {
        let show = match mode {
            Mode::Compact => Some(COMPACT),
            Mode::Expanded | Mode::Panel => Some(EXPANDED),
            Mode::Alert => Some(ALERT),
            Mode::Idle if has_idle => Some(IDLE),
            Mode::Idle => None,
        };
        for (i, layer) in st.layers.iter_mut().enumerate() {
            if Some(i) == show {
                continue;
            }
            let current = layer.opacity.value(now);
            if current > 0.0 || layer.visible {
                // Fade out; once invisible it's hidden for real (screen readers would otherwise read its old text).
                layer.hit = false;
                layer.opacity = Tween::new(current, 0.0, now, 0.110, Ease::Linear);
            }
        }
        let Some(i) = show else { return };
        // Content is laid out at its final size from the start; the animating pill clips it.
        let delay = if matches!(mode, Mode::Compact | Mode::Idle) { 0.06 } else { 0.14 };
        let layer = &mut st.layers[i];
        let before = layer.opacity.value(now);
        layer.size = size;
        layer.hit = true;
        layer.visible = true;
        layer.opacity = Tween::new(0.0, 1.0, now + delay, 0.220, Ease::CubicOut).holding(before);
        layer.scale = Tween::new(0.9, 1.0, now + delay, 0.380, Ease::BackOut(0.35));
        // The blur-in is skipped on battery saver.
        layer.blur = if power::saving() { Tween::fixed(0.0) } else { Tween::new(10.0, 0.0, now + delay, 0.260, Ease::CubicOut) };
    }

    /// Tells the activity in the compact pill when the mouse is over it (music shows its buttons then).
    fn update_hover_target(&self) {
        let target = {
            let st = self.st.borrow();
            if st.hovering && st.mode == Mode::Compact { self.manager.active().first().cloned() } else { None }
        };
        let previous = {
            let st = self.st.borrow();
            st.hover_target.clone()
        };
        let same = match (&previous, &target) {
            (None, None) => true,
            (Some(a), Some(b)) => addr(a) == addr(b),
            _ => false,
        };
        if same {
            return;
        }
        self.st.borrow_mut().hover_target = target.clone();
        if let Some(p) = previous {
            p.on_hover(false);
        }
        if let Some(t) = target {
            t.on_hover(true);
        }
    }

    fn now(&self) -> f64 {
        self.st.borrow().clock_start.elapsed().as_secs_f64()
    }

    /// Starts the frame clock (until the springs and fades settle).
    fn kick(&self) {
        set_animating(true);
        self.request_redraw();
    }

    /// Draws a new frame soon (content changed, e.g. a countdown).
    pub fn request_redraw(&self) {
        if ANIMATING.load(Ordering::SeqCst) {
            return; // the next frame draws it anyway
        }
        {
            let mut st = self.st.borrow_mut();
            if st.redraw_queued {
                return;
            }
            st.redraw_queued = true;
        }
        rt::defer(|| {
            if let Some(i) = get() {
                i.st.borrow_mut().redraw_queued = false;
                i.draw();
            }
        });
    }

    fn on_frame(&self) {
        FRAME_PENDING.store(false, Ordering::SeqCst);
        let settled = {
            let mut st = self.st.borrow_mut();
            let now = Instant::now();
            let dt = st.last_frame.map_or(0.0, |l| now.duration_since(l).as_secs_f64().min(1.0 / 20.0));
            st.last_frame = Some(now);
            let mut settled = true;
            let st = &mut *st;
            for s in [&mut st.width, &mut st.height, &mut st.radius, &mut st.bubble, &mut st.press] {
                s.step(dt);
                settled &= s.is_settled();
            }
            settled
        };
        self.draw();
        let now = self.now();
        let st = self.st.borrow();
        let tweens_done = st.layers.iter().all(|l| l.opacity.is_done(now) && l.scale.is_done(now) && l.blur.is_done(now))
            && st.press_tweens.values().all(|t| t.is_done(now));
        if settled && tweens_done && !st.content_animating {
            drop(st);
            set_animating(false);
            self.st.borrow_mut().last_frame = None;
        }
    }

    // ───────────────────────────── drawing ─────────────────────────────

    /// Pixels per DIP on the island's display, and the island's size setting.
    fn scale(&self) -> (f64, Option<di4w_core::displays::Display>) {
        let display = di4w_core::displays::choose(&self.settings.borrow().monitor);
        let dpi = display.as_ref().map_or(96, |d| {
            let (mut x, mut y) = (96u32, 96u32);
            unsafe {
                let _ = GetDpiForMonitor(HMONITOR(d.handle as *mut _), MDT_EFFECTIVE_DPI, &mut x, &mut y);
            }
            x
        });
        let scale = self.settings.borrow().scale.clamp(0.5, 3.0);
        (dpi as f64 / 96.0 * scale, display)
    }

    /// Where the window goes: top-center of its display, at the top of the work area (below a taskbar docked at the
    /// top, at the very top otherwise). Physical pixels.
    fn anchor(&self, pixel_width: i32) -> (i32, i32) {
        let display = di4w_core::displays::choose(&self.settings.borrow().monitor);
        match display {
            Some(d) => (d.bounds.left + (d.bounds.width() - pixel_width) / 2, d.work.top),
            None => (0, 0),
        }
    }

    fn window_size(&self) -> (i32, i32, f64) {
        let (s, _) = self.scale();
        ((CANVAS_WIDTH * s).round() as i32, (CANVAS_HEIGHT * s).round() as i32, s)
    }

    /// Snaps the window to the top-center of its display (main display, the chosen one, or the one with the mouse).
    fn position(&self) {
        let (w, h, _) = self.window_size();
        let (x, y) = self.anchor(w);
        unsafe {
            let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), x, y, w, h, SWP_NOACTIVATE);
        }
    }

    fn draw(&self) {
        if !self.is_visible() {
            return;
        }
        let (w_px, h_px, px) = self.window_size();
        let (x, y) = self.anchor(w_px);
        let now = self.now();
        let light = colors::light();
        let saving = power::saving();
        let top_offset = self.settings.borrow().top_offset;
        let pill_bg = colors::pill_background();

        // Geometry (ApplyFrame): a press squeezes the island by a few points, around its centre.
        let (outline, pill, bubble_rect, bubble_scale, bubble_opacity, neck, indicators_x, top, keyline_color, bubble_width) = {
            let st = self.st.borrow();
            let press = st.press.value;
            let w = (st.width.value - press * (10.0f64).min(st.width.value * 0.04)).max(20.0);
            let h = (st.height.value - press * 2.5).max(20.0);
            let r = st.radius.value.clamp(0.0, w.min(h) / 2.0);
            let mut left = (CANVAS_WIDTH - w) / 2.0;
            let top = top_offset + (st.height.value - h) / 2.0;

            // When the bubble is out, shift the pill left so pill + bubble stay centered as a group.
            let split = st.bubble.value.max(0.0);
            let bw = st.bubble_width;
            left -= split.min(1.0) * (bw + BUBBLE_GAP) / 2.0;
            let outline = shape::rounded(w, h, r).transformed(1.0, left, top);

            // The bubble buds off the island's trailing end like a drop: it starts inside the pill, small, and moves
            // out to its place while growing; until it lets go, a narrowing neck of black joins the two.
            let pill_right = left + w;
            let bubble_left = pill_right + BUBBLE_GAP * split - (1.0 - split.min(1.0)) * bw * 0.8;
            let scale = 0.45 + 0.55 * split;
            let bubble_rect = Rect::new(bubble_left, top + (h - BUBBLE_SIZE) / 2.0, bw, BUBBLE_SIZE);
            // Only while the bubble is out: tucked inside the pill, the neck would cover the trailing content.
            let neck = if split > 0.01 { neck(pill_right, bubble_left + bw * (1.0 - scale) / 2.0, top + h / 2.0, h.min(BUBBLE_SIZE * scale) / 2.0) } else { None };
            let indicators_x = pill_right + BUBBLE_GAP + split.min(1.0) * (bw + BUBBLE_GAP);
            let keyline = if light {
                LIGHT_KEYLINE
            } else {
                match st.keyline {
                    Some(c) => Color::argb(0x5C, c.r, c.g, c.b),
                    None => NEUTRAL_KEYLINE,
                }
            };
            (outline, Rect::new(left, top, w, h), bubble_rect, scale.max(0.0), if split > 0.01 { 1.0 } else { 0.0 }, neck, indicators_x, top, keyline, bw)
        };

        // Build and lay out the visible layers.
        let mut layers: Vec<(El, f64, f64, f64, Rect, bool)> = Vec::new(); // (tree, opacity, scale, blur, layer rect, hit)
        let mut animating_content = false;
        {
            let st = self.st.borrow();
            for (i, layer) in st.layers.iter().enumerate() {
                let opacity = layer.opacity.value(now);
                if !layer.visible || opacity <= 0.001 {
                    continue;
                }
                let content = self.layer_view(i, &layer.content, layer.size, &mut animating_content);
                let lw = layer.size.w;
                let lh = layer.size.h;
                let mut root = El::boxed(vec![El::boxed(vec![content]).size(lw, lh).align(Align::Center, Align::Start)]);
                root.layout(pill);
                let lrect = Rect::new(pill.x + (pill.w - lw) / 2.0, pill.y, lw, lh);
                layers.push((root, opacity, layer.scale.value(now), if saving { 0.0 } else { layer.blur.value(now) }, lrect, layer.hit));
            }
        }
        // Fully faded layers are hidden for real.
        {
            let mut st = self.st.borrow_mut();
            for layer in st.layers.iter_mut() {
                if layer.visible && !layer.hit && layer.opacity.is_done(now) && layer.opacity.to <= 0.0 {
                    layer.visible = false;
                }
            }
        }

        let bubble_activity = self.st.borrow().bubble_content.clone();
        let neck_visible = neck.is_some();
        let bubble_el = if bubble_opacity > 0.0 {
            let content = bubble_activity.map(|a| a.bubble().align(Align::Center, Align::Center)).unwrap_or_else(El::empty);
            let mut b = El::boxed(vec![content]).size(bubble_width, BUBBLE_SIZE).radius(18.5).bg(pill_bg);
            if !neck_visible {
                b = b.border(if light { LIGHT_KEYLINE } else { NEUTRAL_KEYLINE }, 1.0); // the key line would draw a seam across the neck
            }
            let mut root = El::boxed(vec![b.align(Align::Start, Align::Start)]);
            root.layout(Rect::new(bubble_rect.x, bubble_rect.y, CANVAS_WIDTH, CANVAS_HEIGHT));
            Some(root)
        } else {
            None
        };

        // Privacy dots sit after the pill (and the bubble while it's out), level with the compact row.
        let dots: Vec<Dot> = self.dots.borrow().iter().filter(|d| d.visible).cloned().collect();
        let dots_height = if dots.is_empty() { 0.0 } else { 8.0 };
        let dots_y = top + (COMPACT_HEIGHT - dots_height) / 2.0;
        let dot_rects: Vec<(Rect, Option<String>, Color)> =
            dots.iter().enumerate().map(|(i, d)| (Rect::new(indicators_x + i as f64 * 14.0, dots_y, 8.0, 8.0), d.tooltip.clone(), d.color)).collect();

        let paint_state = {
            let st = self.st.borrow();
            PaintState { hovered_button: st.hovered_button, press: st.press_tweens.iter().map(|(k, t)| (*k, t.value(now))).collect() }
        };

        let mut renderer = self.renderer.borrow_mut();
        if let Some(renderer) = renderer.as_mut() {
            let mut draw = |p: &mut ui::render::Painter| {
                // Blurred layers are recorded first (nothing may be pushed while recording).
                let mut recorded = Vec::new();
                for (root, _, scale, blur, lrect, _) in &layers {
                    if *blur > 0.05 {
                        let origin = (lrect.x + lrect.w * 0.5, lrect.y + lrect.h * 0.3);
                        recorded.push(p.record(&mut |p| {
                            let saved = p.push_transform(scale_around(*scale, *scale, origin.0, origin.1));
                            p.paint(root);
                            p.restore_transform(saved);
                        }));
                    } else {
                        recorded.push(None);
                    }
                }
                // Soft shadow under the light island only: the dark one is pure black like Apple's, with a key line.
                if light && !saving {
                    p.shadow(&outline, Color::BLACK.faded(0.45 * 0.35), 6.0, 2.12, 2.12);
                }
                p.push_clip(&outline);
                p.fill(&outline, pill_bg.into());
                for ((root, opacity, scale, blur, lrect, _), list) in layers.iter().zip(recorded.iter()) {
                    p.push_opacity(*opacity);
                    match list {
                        // WPF's blur radius is about three standard deviations.
                        Some(list) => p.draw_blurred(list, blur / 3.0),
                        None => {
                            let origin = (lrect.x + lrect.w * 0.5, lrect.y + lrect.h * 0.3);
                            let saved = p.push_transform(scale_around(*scale, *scale, origin.0, origin.1));
                            p.paint(root);
                            p.restore_transform(saved);
                        }
                    }
                    p.pop();
                }
                p.pop();
                // Apple's key line: a hairline around the island that tells it apart from a dark background.
                p.stroke(&outline, keyline_color, 1.0, false);
                if let Some(neck) = &neck {
                    p.fill(neck, pill_bg.into());
                }
                if let Some(b) = &bubble_el {
                    let c = bubble_rect.center();
                    let saved = p.push_transform(scale_around(bubble_scale, bubble_scale, c.x, c.y));
                    p.paint(b);
                    p.restore_transform(saved);
                }
                for (r, _, color) in &dot_rects {
                    p.ellipse(*r, Some(*color), None);
                }
            };
            let pos = POINT { x, y };
            if let Err(e) = renderer.present(self.hwnd, pos, w_px as u32, h_px as u32, px, &paint_state, &mut draw) {
                di4w_core::log::error_with("Drawing the island failed", e);
            }
        }
        drop(renderer);

        let mut st = self.st.borrow_mut();
        st.content_animating = animating_content;
        if animating_content {
            set_animating(true);
        }
        st.pill_outline = outline;
        st.pill_rect = pill;
        st.bubble_rect = bubble_rect;
        st.hit_layers = layers.into_iter().filter(|l| l.5).map(|l| l.0).collect();
        st.bubble_el = bubble_el;
        st.indicator_rects = dot_rects.into_iter().map(|(r, t, _)| (r, t)).collect();
    }

    /// The view of one layer.
    fn layer_view(&self, index: usize, content: &Content, size: Size, animating: &mut bool) -> El {
        match content {
            Content::Compact(primary, split) => {
                // Compact: leading (left) and trailing (right) around the "camera", snug against the rounded ends
                // with even margins (Apple's concentric placement).
                let leading = if *split { primary.bubble() } else { primary.leading() };
                let mut children = vec![leading.halign(Align::Start).valign(Align::Center).margin(10.0, 0.0, 0.0, 0.0)];
                if !*split {
                    children.push(primary.trailing().halign(Align::End).valign(Align::Center).margin(0.0, 0.0, 12.0, 0.0));
                }
                El::boxed(children)
            }
            Content::Idle if index == IDLE => {
                let idle = self.idle.borrow();
                let mut children = Vec::new();
                if let Some(f) = &idle.0 {
                    children.push(f().halign(Align::Start).valign(Align::Center).margin(12.0, 0.0, 0.0, 0.0));
                }
                if let Some(f) = &idle.1 {
                    children.push(f().halign(Align::End).valign(Align::Center).margin(0.0, 0.0, 13.0, 0.0));
                }
                El::boxed(children)
            }
            Content::Expanded(a) => a.expanded(),
            Content::Panel(p) => (p.view)(),
            Content::Alert(a) => {
                let view = (a.content)();
                *animating |= ANIMATED_CONTENT.with(|c| c.replace(false));
                view
            }
            _ => {
                let _ = size;
                El::empty()
            }
        }
    }

    // ───────────────────────────── input ─────────────────────────────

    fn to_canvas(&self, lparam: LPARAM) -> Point {
        let x = (lparam.0 & 0xFFFF) as i16 as f64;
        let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as f64;
        let (_, _, px) = self.window_size();
        Point::new(x / px, y / px)
    }

    fn over_pill(&self, p: Point) -> bool {
        let st = self.st.borrow();
        if !st.pill_rect.contains(p) {
            return false;
        }
        self.renderer.borrow().as_ref().is_some_and(|r| r.contains(&st.pill_outline, p))
    }

    fn over_bubble(&self, p: Point) -> bool {
        let st = self.st.borrow();
        st.bubble_hit && st.bubble_rect.contains(p)
    }

    /// The innermost button under the point (in the pill's layers, or the bubble).
    fn button_at(&self, p: Point) -> Option<(u64, crate::ui::el::Action)> {
        let st = self.st.borrow();
        for root in st.hit_layers.iter().rev() {
            if let Some(b) = root.hits(p).into_iter().rev().find_map(|e| e.button.clone()) {
                return Some((b.key, b.action));
            }
        }
        None
    }

    fn mouse_move(self: &Rc<Self>, p: Point) {
        if !self.st.borrow().tracking {
            let mut track = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: self.hwnd, dwHoverTime: 0 };
            unsafe {
                let _ = TrackMouseEvent(&mut track);
            }
            self.st.borrow_mut().tracking = true;
        }
        let over = self.over_pill(p);
        let hovering = self.st.borrow().hovering;
        if over && !hovering {
            self.pill_enter();
        } else if !over && hovering {
            self.pill_leave();
        }

        // Button hover feedback and the cursor.
        let button = if over { self.button_at(p).map(|b| b.0) } else { None };
        let cursor = if over {
            let st = self.st.borrow();
            let mut c = Cursor::Hand; // the pill's own cursor
            for root in &st.hit_layers {
                for e in root.hits(p) {
                    if e.cursor != Cursor::Inherit {
                        c = e.cursor;
                    }
                }
            }
            c
        } else if self.over_bubble(p) {
            Cursor::Hand
        } else {
            Cursor::Inherit
        };
        let changed = {
            let mut st = self.st.borrow_mut();
            st.cursor = cursor;
            let changed = st.hovered_button != button;
            st.hovered_button = button;
            changed
        };
        if changed {
            self.request_redraw();
        }
        crate::tooltip::hover(self.hwnd, self.tooltip_at(p));
    }

    fn tooltip_at(&self, p: Point) -> Option<String> {
        let st = self.st.borrow();
        for (r, tip) in &st.indicator_rects {
            if r.contains(p) {
                return tip.clone();
            }
        }
        if !st.pill_rect.contains(p) {
            return None;
        }
        for root in st.hit_layers.iter().rev() {
            if let Some(t) = root.hits(p).into_iter().rev().find_map(|e| e.tooltip.clone()) {
                return Some(t);
            }
        }
        None
    }

    fn pill_enter(self: &Rc<Self>) {
        {
            let mut st = self.st.borrow_mut();
            st.hovering = true;
            st.collapse = None;
        }
        self.manager.set_hold(true);
        self.manager.alert_hovered();
        self.refresh();
    }

    fn pill_leave(self: &Rc<Self>) {
        self.st.borrow_mut().hovering = false;
        self.set_press(false);
        self.manager.set_hold(false);
        let open = {
            let st = self.st.borrow();
            st.expanded || st.panel.is_some()
        };
        if open {
            let weak = Rc::downgrade(self);
            self.st.borrow_mut().collapse = Some(Timer::once(Duration::from_millis(900), move || {
                if let Some(i) = weak.upgrade() {
                    i.st.borrow_mut().collapse = None;
                    i.collapse();
                }
            }));
        }
        self.refresh();
    }

    fn mouse_leave(self: &Rc<Self>) {
        {
            let mut st = self.st.borrow_mut();
            st.tracking = false;
            st.hovered_button = None;
            st.cursor = Cursor::Inherit;
        }
        if self.st.borrow().hovering {
            self.pill_leave();
        }
        crate::tooltip::hover(self.hwnd, None);
        self.request_redraw();
    }

    fn set_press(&self, down: bool) {
        self.st.borrow_mut().press.target = if down { 1.0 } else { 0.0 };
        self.kick();
    }

    fn button_press(&self, key: u64, down: bool) {
        let now = self.now();
        let mut st = self.st.borrow_mut();
        let current = st.press_tweens.get(&key).map_or(1.0, |t| t.value(now));
        // Shrink to 0.88 in 80 ms; spring back with a little overshoot (BackEase 0.6) in 250 ms.
        let tween = if down { Tween::new(current, 0.88, now, 0.08, Ease::Linear) } else { Tween::new(current, 1.0, now, 0.25, Ease::BackOut(0.6)) };
        st.press_tweens.insert(key, tween);
        st.press_tweens.retain(|k, t| *k == key || !t.is_done(now) || (t.to - 1.0).abs() > 1e-9);
        drop(st);
        self.kick();
    }

    fn left_down(self: &Rc<Self>, p: Point) {
        unsafe {
            SetCapture(self.hwnd);
        }
        if self.over_pill(p) {
            if let Some((key, _)) = self.button_at(p) {
                self.st.borrow_mut().pressed_button = Some(key);
                self.button_press(key, true);
            } else {
                self.set_press(true);
            }
        }
    }

    fn left_up(self: &Rc<Self>, p: Point) {
        unsafe {
            let _ = ReleaseCapture();
        }
        let pressed = self.st.borrow_mut().pressed_button.take();
        if let Some(key) = pressed {
            self.button_press(key, false);
            // A button clicks when released over itself.
            if self.over_pill(p)
                && let Some((k, action)) = self.button_at(p)
                && k == key {
                    action();
                    self.request_redraw();
                }
            return;
        }
        if self.over_bubble(p) {
            let secondary = self.st.borrow().secondary.clone();
            if let Some(s) = secondary {
                self.manager.pin(&s);
            }
            return;
        }
        if !self.over_pill(p) {
            self.set_press(false);
            return;
        }
        self.set_press(false);
        // An element that handles its own click (album art, a Recent row, the seek bar).
        let handler = {
            let st = self.st.borrow();
            st.hit_layers.iter().rev().find_map(|root| root.hits(p).into_iter().rev().find_map(|e| e.click.clone().map(|c| (c, e.rect))))
        };
        if let Some((click, rect)) = handler {
            click(p, rect);
            return;
        }
        self.pill_click();
    }

    fn pill_click(self: &Rc<Self>) {
        let mode = self.st.borrow().mode;
        match mode {
            Mode::Alert => {
                let alert = self.manager.current_alert();
                if let Some(f) = alert.as_ref().and_then(|a| a.on_click.clone()) {
                    f();
                }
                self.manager.dismiss_alert(None);
            }
            Mode::Compact => {
                self.st.borrow_mut().expanded = true;
                self.refresh();
            }
            Mode::Idle => {
                let factory = self.idle_panel.borrow().clone();
                if let Some(f) = factory {
                    self.open_panel(f());
                }
            }
            _ => {}
        }
    }

    fn right_up(self: &Rc<Self>, p: Point) {
        if !self.over_pill(p) && !self.over_bubble(p) {
            return;
        }
        let menu = self.context_menu.borrow().clone();
        let Some(menu) = menu else { return };
        // Below the pill, its left edges aligned (WPF's PlacementMode.Bottom).
        let (pill, px) = {
            let st = self.st.borrow();
            (st.pill_rect, self.window_size().2)
        };
        let mut r = RECT::default();
        unsafe {
            let _ = GetWindowRect(self.hwnd, &mut r);
        }
        menu(r.left + (pill.x * px).round() as i32, r.top + (pill.bottom() * px).round() as i32);
    }

    fn wheel(&self, p: Point, delta: i16) {
        let handler = {
            let st = self.st.borrow();
            if !st.pill_rect.contains(p) {
                return;
            }
            st.hit_layers.iter().rev().find_map(|root| root.hits(p).into_iter().rev().find_map(|e| e.wheel.clone()))
        };
        if let Some(h) = handler {
            h(if delta > 0 { 1 } else { -1 });
            self.request_redraw();
        } else {
            // A scrolling list (Recent) under the mouse.
            let st = self.st.borrow();
            for root in &st.hit_layers {
                for e in root.hits(p) {
                    if let Kind::Scroll { offset, .. } = &e.kind {
                        offset.set(offset.get() - delta as f64 / 120.0 * 48.0);
                    }
                }
            }
            drop(st);
            self.request_redraw();
        }
    }

    // ───────────────────────────── accessibility ─────────────────────────────

    /// Gives screen readers a description of what the island shows ("Timer, 4:32", "Download complete,
    /// photos.zip") and announces new alerts, which otherwise only appear visually.
    fn update_accessible_name(&self) {
        let mode = self.st.borrow().mode;
        let words = |el: El| ui::texts(&el).join(", ");
        let join = |parts: Vec<String>| parts.into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join(", ");
        let alert = self.manager.current_alert();
        let primary = self.manager.active().first().cloned();
        let text = match (mode, &alert, &primary) {
            (Mode::Alert, Some(a), _) => words((a.content)()),
            (Mode::Panel, ..) => {
                let p = self.st.borrow().panel.clone();
                p.map(|p| words((p.view)())).unwrap_or_default()
            }
            (Mode::Expanded, _, Some(p)) => words(p.expanded()),
            (Mode::Compact, _, Some(p)) => {
                let split = self.st.borrow().secondary.is_some();
                let leading = if split { p.bubble() } else { p.leading() };
                let trailing = if split { String::new() } else { words(p.trailing()) };
                join(vec![p.name(), p.accessible_detail().unwrap_or_default(), words(leading), trailing])
            }
            _ => {
                let idle = self.idle.borrow();
                join(vec![
                    l::t("Dynamic Island").to_string(),
                    idle.0.as_ref().map(|f| words(f())).unwrap_or_default(),
                    idle.1.as_ref().map(|f| words(f())).unwrap_or_default(),
                ])
            }
        };
        let announce = {
            let mut st = self.st.borrow_mut();
            let current = alert.as_ref().map_or(0, |a| Rc::as_ptr(a) as usize);
            let announce = mode == Mode::Alert && st.announced != current;
            if mode == Mode::Alert {
                st.announced = current;
            }
            announce
        };
        accessibility::update(self.hwnd, &text, announce);
    }

    fn keep_on_top(&self) {
        if self.is_visible() {
            unsafe {
                let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            }
        }
    }

    fn housekeeping(self: &Rc<Self>) {
        self.update_accessible_name(); // keeps changing text (countdowns) current for screen readers

        // Require two checks in a row so a window that covers the screen for a moment doesn't make it blink away.
        let hide_in_fullscreen = self.settings.borrow().hide_in_fullscreen;
        let fullscreen = hide_in_fullscreen && !self.keep_visible_in_fullscreen.get() && crate::interop::is_fullscreen_app_running();
        let ticks = {
            let mut st = self.st.borrow_mut();
            st.fullscreen_ticks = if fullscreen { st.fullscreen_ticks + 1 } else { 0 };
            st.fullscreen_ticks
        };
        let hide = ticks >= 2;
        let visible = self.is_visible();
        if hide && visible {
            self.hide();
        } else if !hide && !visible && !self.paused.get() {
            self.show();
        }

        if !hide && self.is_visible() {
            self.keep_on_top();
            // Snap back if the position drifted (a missed display change), and follow the mouse to another display
            // when the island is set to.
            let mut r = RECT::default();
            unsafe {
                let _ = GetWindowRect(self.hwnd, &mut r);
            }
            let (w, h, _) = self.window_size();
            if (r.left, r.top) != self.anchor(r.right - r.left) || r.right - r.left != w || r.bottom - r.top != h {
                self.position();
                self.request_redraw();
            }
        }
    }
}

thread_local! {
    /// Set by alert content that animates (the ringing timer's pulse), so frames keep coming while it shows.
    static ANIMATED_CONTENT: Cell<bool> = const { Cell::new(false) };
}

/// Called by a view while it's built: it animates, so the island keeps drawing frames.
pub fn animating_content() {
    ANIMATED_CONTENT.with(|c| c.set(true));
}

/// The black bridge between the island and the bubble while they're close: full height when they touch, pinching
/// to nothing at [`NECK_REACH`] apart.
fn neck(pill_right: f64, bubble_left: f64, center_y: f64, half_height: f64) -> Option<Path> {
    let gap = bubble_left - pill_right;
    let t = (1.0 - gap / NECK_REACH).clamp(0.0, 1.0);
    if t <= 0.0 || half_height < 1.0 {
        return None;
    }
    let x0 = pill_right - 8.0;
    let x1 = bubble_left + 8.0;
    let mid = (x0 + x1) / 2.0;
    let end = half_height * 0.8;
    let pinch = end * t * t; // the waist thins faster than the ends as they part
    use crate::ui::geom::Seg;
    Some(Path {
        segs: vec![
            Seg::Move(Point::new(x0, center_y - end)),
            Seg::Quad(Point::new(mid, center_y - pinch), Point::new(x1, center_y - end)),
            Seg::Line(Point::new(x1, center_y + end)),
            Seg::Quad(Point::new(mid, center_y + pinch), Point::new(x0, center_y + end)),
            Seg::Close,
        ],
    })
}

unsafe extern "system" fn foreground_changed(_: HWINEVENTHOOK, _: u32, _: HWND, _: i32, _: i32, _: u32, _: u32) {
    rt::guard(|| {
        if let Some(i) = get() {
            i.keep_on_top();
        }
    });
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let Some(island) = get() else { return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) } };
    rt::guard(|| {
        // Every message goes to the add-ons that listen (device changes, clipboard, raw input, WM_COPYDATA).
        let hooks: Vec<Rc<dyn Fn(u32, WPARAM, LPARAM)>> = island.hooks.borrow().iter().map(|(_, h)| h.clone()).collect();
        for h in hooks {
            h(msg, wparam, lparam);
        }
    });
    let handled = rt::guard(|| -> Option<LRESULT> {
        match msg {
            WM_FRAME => {
                island.on_frame();
                Some(LRESULT(0))
            }
            WM_WINDOWPOSCHANGING => {
                // The island is pinned: whoever moves it (DPI change, Win+arrow, shell, other apps) it stays at the
                // top-center, and it can't be pushed out of the topmost band.
                let pos = unsafe { &mut *(lparam.0 as *mut WINDOWPOS) };
                let mut width = pos.cx;
                if pos.flags.contains(SWP_NOSIZE) || width <= 0 {
                    let mut r = RECT::default();
                    unsafe {
                        let _ = GetWindowRect(hwnd, &mut r);
                    }
                    width = r.right - r.left;
                }
                let (x, y) = island.anchor(width);
                pos.x = x;
                pos.y = y;
                pos.flags &= !SWP_NOMOVE;
                if !pos.flags.contains(SWP_NOZORDER) {
                    pos.hwndInsertAfter = HWND_TOPMOST;
                }
                Some(LRESULT(0))
            }
            WM_MOUSEACTIVATE => Some(LRESULT(MA_NOACTIVATE as isize)),
            WM_SETCURSOR => {
                let cursor = island.st.borrow().cursor;
                let id = match cursor {
                    Cursor::Hand => IDC_HAND,
                    Cursor::SizeNs => IDC_SIZENS,
                    Cursor::Inherit => IDC_ARROW,
                };
                unsafe {
                    SetCursor(LoadCursorW(None, id).ok());
                }
                Some(LRESULT(1))
            }
            WM_MOUSEMOVE => {
                island.mouse_move(island.to_canvas(lparam));
                Some(LRESULT(0))
            }
            WM_MOUSELEAVE => {
                island.mouse_leave();
                Some(LRESULT(0))
            }
            WM_LBUTTONDOWN => {
                island.left_down(island.to_canvas(lparam));
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                island.left_up(island.to_canvas(lparam));
                Some(LRESULT(0))
            }
            WM_RBUTTONUP => {
                island.right_up(island.to_canvas(lparam));
                Some(LRESULT(0))
            }
            WM_MOUSEWHEEL => {
                // Screen coordinates here.
                let mut pt = POINT { x: (lparam.0 & 0xFFFF) as i16 as i32, y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32 };
                unsafe {
                    let _ = windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut pt);
                }
                let (_, _, px) = island.window_size();
                island.wheel(Point::new(pt.x as f64 / px, pt.y as f64 / px), ((wparam.0 >> 16) & 0xFFFF) as i16);
                Some(LRESULT(0))
            }
            WM_HOTKEY if wparam.0 as i32 == HIDE_HOTKEY => {
                let f = island.hotkey_pressed.borrow().clone();
                if let Some(f) = f {
                    f();
                }
                Some(LRESULT(0))
            }
            WM_HOTKEY if wparam.0 as i32 == EXPAND_HOTKEY => {
                island.toggle_expanded();
                Some(LRESULT(0))
            }
            WM_DISPLAYCHANGE | WM_SETTINGCHANGE | WM_DPICHANGED => {
                let i = island.clone();
                rt::defer(move || {
                    i.position();
                    i.request_redraw();
                });
                if msg == WM_DPICHANGED { Some(LRESULT(0)) } else { None }
            }
            WM_GETOBJECT => accessibility::get_object(hwnd, wparam, lparam),
            WM_CLOSE => {
                // Closing the island window (installer upgrade/uninstall, Windows shutting down) quits the whole app
                // cleanly, so the tray icon, port and settings are released properly.
                let f = island.on_close.borrow().clone();
                if let Some(f) = f {
                    rt::defer(move || f());
                }
                Some(LRESULT(0))
            }
            WM_QUERYENDSESSION => Some(LRESULT(1)),
            WM_ENDSESSION if wparam.0 != 0 => {
                let f = island.on_close.borrow().clone();
                if let Some(f) = f {
                    f();
                }
                Some(LRESULT(0))
            }
            _ => None,
        }
    })
    .flatten();
    handled.unwrap_or_else(|| unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) })
}
