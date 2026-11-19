//! Pomodoro focus timer: work and break periods run back to back with a ring showing progress. Every Nth break is
//! a long one. Lengths come from the settings app.

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
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Focus,
    Break,
    LongBreak,
}

struct State {
    phase: Phase,
    length: Duration,
    completed: i32,
    watch: Watch,
    active: bool,
    tick: Option<Timer>,
}

pub struct FocusActivity {
    manager: Rc<Manager>,
    settings: Rc<RefCell<AppSettings>>,
    st: RefCell<State>,
    me: Weak<FocusActivity>,
}

impl FocusActivity {
    pub fn new(manager: Rc<Manager>, settings: Rc<RefCell<AppSettings>>) -> Rc<FocusActivity> {
        Rc::new_cyclic(|me| FocusActivity {
            manager,
            settings,
            st: RefCell::new(State { phase: Phase::Focus, length: Duration::ZERO, completed: 0, watch: Watch::default(), active: false, tick: None }),
            me: me.clone(),
        })
    }

    fn color(&self) -> Color {
        if self.st.borrow().phase == Phase::Focus { colors::red() } else { colors::green() }
    }

    fn sessions(&self) -> i32 {
        self.settings.borrow().addons.sessions_before_long_break.max(1)
    }

    pub fn start(&self) {
        self.st.borrow_mut().completed = 0;
        self.begin(Phase::Focus);
        self.st.borrow_mut().active = true;
        self.manager.changed();
    }

    pub fn stop(&self) {
        {
            let mut st = self.st.borrow_mut();
            st.tick = None;
            st.watch.reset();
            st.active = false;
        }
        self.manager.changed();
    }

    fn toggle_pause(&self) {
        self.st.borrow_mut().watch.toggle();
        self.update();
    }

    fn skip(&self) {
        self.next(false);
    }

    fn begin(&self, phase: Phase) {
        let minutes = {
            let o = &self.settings.borrow().addons;
            match phase {
                Phase::Focus => o.focus_minutes,
                Phase::Break => o.break_minutes,
                Phase::LongBreak => o.long_break_minutes,
            }
            .max(1)
        };
        let me = self.me.clone();
        {
            let mut st = self.st.borrow_mut();
            st.phase = phase;
            st.length = Duration::from_secs(minutes as u64 * 60);
            st.watch.restart();
            st.tick = Some(Timer::every(Duration::from_millis(250), move || {
                if let Some(f) = me.upgrade() {
                    f.update();
                }
            }));
        }
        self.manager.changed();
        self.update();
    }

    fn next(&self, announce: bool) {
        let next = {
            let mut st = self.st.borrow_mut();
            if st.phase == Phase::Focus {
                st.completed += 1;
                if st.completed % self.sessions() == 0 { Phase::LongBreak } else { Phase::Break }
            } else {
                Phase::Focus
            }
        };
        self.begin(next);

        if !announce {
            return;
        }
        self.chime();
        let (minutes, completed) = {
            let st = self.st.borrow();
            (st.length.as_secs() / 60, st.completed)
        };
        let (title, subtitle) = if next == Phase::Focus {
            (l::t("Back to focus").to_string(), l::f("{0} min · session {1}", &[&minutes, &(completed + 1)]))
        } else {
            (if next == Phase::LongBreak { l::t("Long break") } else { l::t("Break time") }.to_string(), l::f("{0} min · nice work", &[&minutes]))
        };
        let color = self.color();
        let glyph = if next == Phase::Focus { icons::FOCUS } else { "\u{EC32}" };
        self.manager.show_alert(
            Alert::new(move || ui::alert_view(ui::glyph_circle(glyph, color, ICON_SIZE, 22.0), &title, &subtitle, Some(color), None))
                .seconds(5.0)
                .breaks_through(true)
                .rc(),
        );
    }

    fn remaining(&self) -> f64 {
        let st = self.st.borrow();
        st.length.as_secs_f64() - st.watch.elapsed().as_secs_f64()
    }

    fn update(&self) {
        if !self.st.borrow().active {
            return;
        }
        if self.remaining() <= 0.0 {
            self.next(true);
            return;
        }
        super::redraw();
    }

    fn text(&self) -> String {
        ui::format_seconds(self.remaining().max(0.0).ceil())
    }

    fn dim(&self) -> f64 {
        if self.st.borrow().watch.is_running() { 1.0 } else { 0.55 }
    }

    fn progress(&self) -> f64 {
        let length = self.st.borrow().length.as_secs_f64().max(1.0);
        1.0 - self.remaining() / length
    }

    fn label(&self) -> String {
        let (phase, completed) = {
            let st = self.st.borrow();
            (st.phase, st.completed)
        };
        match phase {
            Phase::Focus => l::f("Focus · {0} of {1}", &[&(completed % self.sessions() + 1), &self.settings.borrow().addons.sessions_before_long_break]),
            Phase::Break => l::t("Break").to_string(),
            Phase::LongBreak => l::t("Long break").to_string(),
        }
    }

    fn chime(&self) {
        if !self.settings.borrow().sounds {
            return;
        }
        let wav = sound::media("Windows Notify Calendar.wav");
        if !(wav.exists() && sound::play(&wav)) {
            sound::asterisk();
        }
    }
}

impl Activity for FocusActivity {
    fn name(&self) -> String {
        l::t("Focus").to_string()
    }

    fn priority(&self) -> i32 {
        32
    }

    fn kind(&self) -> String {
        "Focus".into()
    }

    fn is_active(&self) -> bool {
        self.st.borrow().active
    }

    fn leading(&self) -> El {
        icons::ring_on_track(self.progress(), 18.0, 2.6, self.color())
    }

    fn trailing(&self) -> El {
        ui::digits(self.text(), 15.0, SEMIBOLD, self.color()).opacity(self.dim())
    }

    fn bubble(&self) -> El {
        icons::ring_on_track(self.progress(), 22.0, 2.4, self.color())
    }

    fn expanded(&self) -> El {
        let color = self.color();
        let paused = !self.st.borrow().watch.is_running();
        let (a, b, c) = (self.me.clone(), self.me.clone(), self.me.clone());
        let pause = ui::circle_button(key("focus.pause"), icons::shape(if paused { icons::play() } else { icons::pause() }, 15.0, color), colors::button_bg(), 48.0, move || {
            if let Some(f) = a.upgrade() {
                f.toggle_pause();
            }
        }, Some(l::t("Pause / resume").into()));
        let skip = ui::circle_button(key("focus.skip"), icons::shape(icons::next(), 14.0, colors::primary()), colors::button_bg(), 48.0, move || {
            if let Some(f) = b.upgrade() {
                f.skip();
            }
        }, Some(l::t("Skip to next").into()))
        .margin(10.0, 0.0, 0.0, 0.0);
        let stop = ui::circle_button(key("focus.stop"), icons::stroke(icons::close(), 12.0, colors::primary(), 2.4), colors::track(), 48.0, move || {
            if let Some(f) = c.upgrade() {
                f.stop();
            }
        }, Some(l::t("Stop").into()))
        .margin(10.0, 0.0, 0.0, 0.0);
        let buttons = El::hstack(vec![pause, skip, stop]).valign(Align::Center);
        let right = El::vstack(vec![
            ui::text(self.label(), 14.0, SEMIBOLD, color).halign(Align::End),
            ui::digits(self.text(), 44.0, LIGHT, color).opacity(self.dim()).halign(Align::End).margin(0.0, -6.0, 0.0, -4.0),
        ])
        .align(Align::End, Align::Center);
        El::grid(vec![Len::Auto, Len::Star(1.0)], vec![], vec![(0, 0, buttons), (0, 1, right)]).margin(18.0, 16.0, 22.0, 16.0)
    }

    fn keyline(&self) -> Option<Color> {
        Some(self.color())
    }

    fn expanded_size(&self) -> Size {
        Size::new(ALERT_WIDTH, 100.0)
    }
}
