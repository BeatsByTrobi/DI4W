//! Countdown timer: orange icon + orange countdown in the pill, iOS-style controls when expanded, and a ringing
//! alert (with a looping alarm sound) when it's done.

pub mod picker;
pub mod recent;

use super::{Watch, sound};
use crate::model::{Activity, Alert, Manager};
use crate::rt::Timer;
use crate::ui::colors::{self, Color};
use crate::ui::el::{Align, El, Len, key};
use crate::ui::geom::Size;
use crate::ui::{self, ALERT_WIDTH, ICON_SIZE, LIGHT, SEMIBOLD, icons};
use di4w_core::{AppSettings, l};
use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

struct State {
    duration: Duration,
    watch: Watch,
    active: bool,
    tick: Option<Timer>,
}

pub struct TimerActivity {
    manager: Rc<Manager>,
    settings: Rc<RefCell<AppSettings>>,
    st: RefCell<State>,
    me: Weak<TimerActivity>,
}

impl TimerActivity {
    pub fn new(manager: Rc<Manager>, settings: Rc<RefCell<AppSettings>>) -> Rc<TimerActivity> {
        Rc::new_cyclic(|me| TimerActivity {
            manager,
            settings,
            st: RefCell::new(State { duration: Duration::ZERO, watch: Watch::default(), active: false, tick: None }),
            me: me.clone(),
        })
    }

    pub fn start(&self, duration: Duration) {
        stop_alarm();
        let me = self.me.clone();
        {
            let mut st = self.st.borrow_mut();
            st.duration = duration;
            st.watch.restart();
            st.tick = Some(Timer::every(Duration::from_millis(200), move || {
                if let Some(t) = me.upgrade() {
                    t.update();
                }
            }));
            st.active = true;
        }
        self.manager.changed();
        self.update();
    }

    pub fn toggle_pause(&self) {
        self.st.borrow_mut().watch.toggle();
        self.update();
    }

    pub fn cancel(&self) {
        {
            let mut st = self.st.borrow_mut();
            st.watch.reset();
            st.tick = None;
            st.active = false;
        }
        self.manager.changed();
    }

    fn remaining(&self) -> f64 {
        let st = self.st.borrow();
        st.duration.as_secs_f64() - st.watch.elapsed().as_secs_f64()
    }

    fn update(&self) {
        if !self.st.borrow().active {
            return;
        }
        if self.remaining() <= 0.0 {
            self.finish();
            return;
        }
        super::redraw();
    }

    /// Round up so "5:00" shows for the whole first second, like iOS.
    fn text(&self) -> String {
        ui::format_seconds(self.remaining().max(0.0).ceil())
    }

    fn paused(&self) -> bool {
        !self.st.borrow().watch.is_running()
    }

    fn dim(&self) -> f64 {
        if self.paused() { 0.55 } else { 1.0 }
    }

    fn finish(&self) {
        let label = {
            let mut st = self.st.borrow_mut();
            st.tick = None;
            st.watch.reset();
            st.active = false;
            ui::format_time(st.duration)
        };
        self.manager.changed();

        let manager = self.manager.clone();
        let started = Instant::now();
        let alert = Alert::cyclic(move |weak| {
            let title_weak = weak.clone();
            let manager_for_stop = manager.clone();
            let mut a = Alert::new(move || {
                // Pulse the icon while ringing (1 → 1.12 and back, 450 ms each way, sine ease).
                crate::island::animating_content();
                let t = started.elapsed().as_secs_f64() / 0.45;
                let phase = t % 2.0;
                let x = if phase < 1.0 { phase } else { 2.0 - phase };
                let pulse = 1.0 + 0.12 * (x * std::f64::consts::FRAC_PI_2).sin();
                let icon = El::boxed(vec![ui::glyph(icons::TIMER, 22.0, colors::orange())])
                    .size(ICON_SIZE, ICON_SIZE)
                    .radius(ICON_SIZE / 2.0)
                    .bg(colors::orange_dim())
                    .scaled(pulse);
                let w = title_weak.clone();
                let m = manager_for_stop.clone();
                let stop = ui::circle_button(
                    key("timer.alarm.stop"),
                    ui::text(l::t("Stop"), 14.0, SEMIBOLD, Color::BLACK),
                    colors::orange(),
                    ICON_SIZE,
                    move || {
                        if let Some(a) = w.upgrade() {
                            m.dismiss_alert(Some(&a));
                        }
                    },
                    None,
                );
                ui::alert_view(icon, l::t("Timer"), &l::f("{0} is done", &[&label]), Some(colors::orange()), Some(stop))
            })
            .sticky()
            .breaks_through(true)
            .keyline(Some(colors::orange()))
            .on_dismissed(stop_alarm);
            a.urgent = false;
            a
        });
        self.manager.show_alert(alert);
        self.play_alarm();
    }

    fn play_alarm(&self) {
        if !self.settings.borrow().sounds {
            return;
        }
        let mut wav = sound::media("Alarm01.wav");
        if !wav.exists() {
            wav = sound::media("Windows Notify System Generic.wav");
        }
        if !sound::play_looping(&wav) {
            di4w_core::log::error("Could not play timer alarm");
        }
    }
}

fn stop_alarm() {
    sound::stop();
}

impl Activity for TimerActivity {
    fn name(&self) -> String {
        l::t("Timer").to_string()
    }

    fn priority(&self) -> i32 {
        30
    }

    fn kind(&self) -> String {
        "Timer".into()
    }

    fn is_active(&self) -> bool {
        self.st.borrow().active
    }

    fn leading(&self) -> El {
        ui::glyph(icons::TIMER, 15.0, colors::orange())
    }

    fn trailing(&self) -> El {
        ui::digits(self.text(), 15.0, SEMIBOLD, colors::orange()).opacity(self.dim())
    }

    /// Minimal: the time left, as the iPhone's Timer shows it (an oval up to 45 wide).
    fn bubble(&self) -> El {
        let text = self.text();
        let size = if text.chars().count() > 5 { 10.5 } else { 13.0 }; // "1:02:03" still fits the oval
        ui::digits(text, size, SEMIBOLD, colors::orange()).opacity(self.dim()).halign(Align::Center)
    }

    /// Layout follows the iOS timer live activity: round controls left, big orange countdown right.
    fn expanded(&self) -> El {
        let me = self.me.clone();
        let me2 = self.me.clone();
        let pause_icon = icons::shape(if self.paused() { icons::play() } else { icons::pause() }, 15.0, colors::orange());
        let pause = ui::circle_button(key("timer.pause"), pause_icon, colors::orange_dim(), 52.0, move || {
            if let Some(t) = me.upgrade() {
                t.toggle_pause();
            }
        }, Some(l::t("Pause / resume").into()));
        let cancel = ui::circle_button(key("timer.cancel"), icons::stroke(icons::close(), 14.0, colors::primary(), 2.4), colors::track(), 52.0, move || {
            if let Some(t) = me2.upgrade() {
                t.cancel();
            }
        }, Some(l::t("Cancel").into()))
        .margin(12.0, 0.0, 0.0, 0.0);
        let buttons = El::hstack(vec![pause, cancel]).valign(Align::Center);
        let right = El::vstack(vec![
            ui::text(l::t("Timer"), 14.0, SEMIBOLD, colors::orange()).halign(Align::End),
            ui::digits(self.text(), 44.0, LIGHT, colors::orange()).opacity(self.dim()).halign(Align::End).margin(0.0, -6.0, 0.0, -4.0),
        ])
        .align(Align::End, Align::Center);
        El::grid(vec![Len::Auto, Len::Star(1.0)], vec![], vec![(0, 0, buttons), (0, 1, right)]).margin(18.0, 16.0, 22.0, 16.0)
    }

    fn keyline(&self) -> Option<Color> {
        Some(colors::orange())
    }

    fn minimal_width(&self) -> f64 {
        45.0
    }

    fn expanded_size(&self) -> Size {
        Size::new(ALERT_WIDTH, 100.0)
    }
}
