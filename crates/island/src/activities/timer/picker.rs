//! The panel that opens when you click the idle island, with four tabs: Timer (preset chips plus a
//! minutes:seconds picker you change with the scroll wheel or +/-), Focus (Pomodoro), Stopwatch and Recent. No text
//! boxes, because the island never takes keyboard focus.

use crate::island::Panel;
use crate::ui::colors::{self, Color};
use crate::ui::el::{Align, Cursor, El, Len, key};
use crate::ui::geom::Size;
use crate::ui::{self, ALERT_WIDTH, LIGHT, NORMAL, SEMIBOLD, icons};
use di4w_core::{AddonSettings, l};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

pub const PANEL_SIZE: Size = Size::new(ALERT_WIDTH, 170.0);
const PRESET_MINUTES: [u64; 6] = [1, 3, 5, 10, 15, 25];

/// What the picker's buttons do.
pub struct Actions {
    pub start_timer: Rc<dyn Fn(Duration)>,
    pub start_focus: Rc<dyn Fn()>,
    pub start_stopwatch: Rc<dyn Fn()>,
    /// Builds the Recent tab (the latest alerts).
    pub recent: Rc<dyn Fn() -> El>,
}

struct State {
    tab: usize,
    minutes: u32,
    seconds: u32,
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// The panel; `start_tab` 3 opens on Recent.
pub fn build(actions: Actions, options: AddonSettings, start_tab: usize) -> Panel {
    let state = Rc::new(RefCell::new(State { tab: start_tab.min(3), minutes: 5, seconds: 0 }));
    let actions = Rc::new(actions);
    let options = Rc::new(options);
    Panel { id: NEXT_ID.fetch_add(1, Ordering::Relaxed), size: PANEL_SIZE, view: Rc::new(move || view(&state, &actions, &options)) }
}

fn modes() -> [(String, &'static str, Color); 4] {
    [
        (l::t("Timer").to_string(), icons::TIMER, colors::orange()),
        (l::t("Focus").to_string(), icons::FOCUS, colors::red()),
        (l::t("Stopwatch").to_string(), icons::TIMER, colors::amber()),
        (l::t("Recent").to_string(), icons::RECENT, colors::blue()),
    ]
}

fn view(state: &Rc<RefCell<State>>, actions: &Rc<Actions>, options: &Rc<AddonSettings>) -> El {
    let modes = modes();
    let tab = state.borrow().tab;
    let (name, glyph, color) = &modes[tab];

    // Header: icon + title on the left, mode switch on the right.
    let title = El::hstack(vec![ui::glyph(glyph, 15.0, *color), ui::text(name.as_str(), 15.0, SEMIBOLD, colors::primary()).margin(8.0, 0.0, 0.0, 0.0)]).halign(Align::Start);
    let tabs: Vec<El> = modes
        .iter()
        .enumerate()
        .map(|(i, (name, _, _))| {
            let selected = i == tab;
            let label = ui::text(name.as_str(), 12.0, SEMIBOLD, if selected { colors::primary() } else { colors::grey() });
            let s = state.clone();
            ui::button(key(&format!("picker.tab.{i}")), label, move || s.borrow_mut().tab = i, Some(if selected { colors::track() } else { Color::TRANSPARENT }), None, Some(24.0), None)
                .padding(10.0, 0.0, 10.0, 0.0)
                .margin(4.0, 0.0, 0.0, 0.0)
        })
        .collect();
    let switcher = El::hstack(tabs).halign(Align::End);
    let header = El::boxed(vec![title, switcher]);

    let content = match tab {
        0 => timer_tab(state, actions),
        1 => focus_tab(actions, options),
        2 => stopwatch_tab(actions),
        _ => (actions.recent)(),
    };
    El::grid(vec![], vec![Len::Auto, Len::Star(1.0)], vec![(0, 0, header), (1, 0, content)]).margin(16.0, 14.0, 16.0, 16.0)
}

fn start_button(id: &str, color: Color, action: impl Fn() + 'static) -> El {
    // "Start" stays untranslated here, as in the C# app.
    ui::circle_button(key(id), ui::text("Start", 14.0, SEMIBOLD, Color::BLACK), color, 56.0, action, None).valign(Align::Center)
}

fn focus_tab(actions: &Rc<Actions>, o: &AddonSettings) -> El {
    let detail = ui::with_text(
        ui::text(
            l::f("then a {0} min break · every {1} break is {2} min", &[&o.break_minutes, &l::ordinal(o.sessions_before_long_break as i64), &o.long_break_minutes]),
            13.0,
            NORMAL,
            colors::grey(),
        ),
        |t| t.wrap = true,
    );
    let text = El::vstack(vec![ui::text(l::f("{0} min focus", &[&o.focus_minutes]), 30.0, LIGHT, colors::primary()), detail]).valign(Align::Center);
    let a = actions.clone();
    let button = start_button("picker.focus.start", colors::red(), move || (a.start_focus)()).margin(12.0, 0.0, 0.0, 0.0);
    El::grid(vec![Len::Star(1.0), Len::Auto], vec![], vec![(0, 0, text), (0, 1, button)]).margin(0.0, 14.0, 0.0, 0.0)
}

fn stopwatch_tab(actions: &Rc<Actions>) -> El {
    let text = El::vstack(vec![
        ui::digits("0:00.0", 34.0, LIGHT, colors::amber()),
        ui::text(l::t("Counts up until you stop it"), 13.0, NORMAL, colors::grey()),
    ])
    .valign(Align::Center);
    let a = actions.clone();
    let button = start_button("picker.stopwatch.start", colors::amber(), move || (a.start_stopwatch)());
    El::grid(vec![Len::Star(1.0), Len::Auto], vec![], vec![(0, 0, text), (0, 1, button)]).margin(0.0, 14.0, 0.0, 0.0)
}

fn timer_tab(state: &Rc<RefCell<State>>, actions: &Rc<Actions>) -> El {
    // Preset chips.
    let chips: Vec<El> = PRESET_MINUTES
        .iter()
        .map(|&m| {
            let a = actions.clone();
            ui::button(
                key(&format!("picker.preset.{m}")),
                ui::text(l::f("{0} min", &[&m]), 13.0, SEMIBOLD, colors::primary()),
                move || (a.start_timer)(Duration::from_secs(m * 60)),
                Some(colors::button_bg()),
                None,
                Some(30.0),
                None,
            )
            .margin(3.0, 0.0, 3.0, 0.0)
        })
        .collect();
    let presets = El::uniform(chips).margin(-3.0, 12.0, -3.0, 0.0);

    // Custom row: [-] mm : ss [+]            [Start]
    let (minutes, seconds) = {
        let s = state.borrow();
        (s.minutes, s.seconds)
    };
    let s = state.clone();
    let min_text = ui::digits(format!("{minutes:02}"), 34.0, LIGHT, colors::primary())
        .cursor(Cursor::SizeNs)
        .bg(Color::TRANSPARENT) // hit-testable
        .tooltip(l::t("Scroll to change"))
        .on_wheel(move |d| {
            let mut s = s.borrow_mut();
            s.minutes = (s.minutes as i64 + d as i64).clamp(0, 180) as u32;
        });
    let s = state.clone();
    let sec_text = ui::digits(format!("{seconds:02}"), 34.0, LIGHT, colors::primary())
        .cursor(Cursor::SizeNs)
        .bg(Color::TRANSPARENT)
        .tooltip(l::t("Scroll to change"))
        .on_wheel(move |d| {
            let mut s = s.borrow_mut();
            let mut seconds = s.seconds as i64 + d as i64 * 5;
            if seconds >= 60 {
                seconds = 0;
                s.minutes = (s.minutes + 1).min(180);
            }
            if seconds < 0 {
                seconds = if s.minutes > 0 { 55 } else { 0 };
                if s.minutes > 0 {
                    s.minutes -= 1;
                }
            }
            s.seconds = seconds as u32;
        });
    let colon = ui::text(":", 34.0, LIGHT, colors::grey()).margin(2.0, -4.0, 2.0, 0.0);
    let s = state.clone();
    let minus = ui::circle_button(key("picker.minus"), ui::text("\u{2212}", 18.0, SEMIBOLD, colors::primary()), colors::button_bg(), 32.0, move || {
        let mut s = s.borrow_mut();
        s.minutes = s.minutes.saturating_sub(1);
    }, Some(l::t("\u{2212}1 min").into()));
    let s = state.clone();
    let plus = ui::circle_button(key("picker.plus"), ui::text("+", 18.0, SEMIBOLD, colors::primary()), colors::button_bg(), 32.0, move || {
        let mut s = s.borrow_mut();
        s.minutes = (s.minutes + 1).min(180);
    }, Some(l::t("+1 min").into()));
    let digits = El::hstack(vec![min_text, colon, sec_text]).margin(12.0, 0.0, 12.0, 0.0);
    let picker = El::hstack(vec![minus, digits, plus]).valign(Align::Center);
    let s = state.clone();
    let a = actions.clone();
    let start = start_button("picker.timer.start", colors::orange(), move || {
        let (m, sec) = {
            let s = s.borrow();
            (s.minutes as u64, s.seconds as u64)
        };
        let t = Duration::from_secs(m * 60 + sec);
        if !t.is_zero() {
            (a.start_timer)(t);
        }
    });
    let custom = El::grid(vec![Len::Auto, Len::Star(1.0), Len::Auto], vec![], vec![(0, 0, picker), (0, 2, start)]).margin(0.0, 14.0, 0.0, 0.0).valign(Align::Center);

    El::grid(vec![], vec![Len::Auto, Len::Star(1.0)], vec![(0, 0, presets), (1, 0, custom)])
}
