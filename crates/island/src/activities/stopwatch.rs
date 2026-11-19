//! Stopwatch that counts up, with pause/resume and stop. Shows tenths when expanded.

use super::Watch;
use crate::model::{Activity, Manager};
use crate::rt::Timer;
use crate::ui::colors::{self, Color};
use crate::ui::el::{Align, El, Len, key};
use crate::ui::geom::Size;
use crate::ui::{self, ALERT_WIDTH, LIGHT, SEMIBOLD, icons};
use di4w_core::l;
use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::time::Duration;

struct State {
    watch: Watch,
    active: bool,
    tick: Option<Timer>,
}

pub struct StopwatchActivity {
    manager: Rc<Manager>,
    st: RefCell<State>,
    me: Weak<StopwatchActivity>,
}

impl StopwatchActivity {
    pub fn new(manager: Rc<Manager>) -> Rc<StopwatchActivity> {
        Rc::new_cyclic(|me| StopwatchActivity { manager, st: RefCell::new(State { watch: Watch::default(), active: false, tick: None }), me: me.clone() })
    }

    pub fn start(&self) {
        {
            let mut st = self.st.borrow_mut();
            st.watch.restart();
            st.tick = Some(Timer::every(Duration::from_millis(100), super::redraw));
            st.active = true;
        }
        self.manager.changed();
    }

    pub fn stop(&self) {
        {
            let mut st = self.st.borrow_mut();
            st.watch.reset();
            st.tick = None;
            st.active = false;
        }
        self.manager.changed();
    }

    fn toggle_pause(&self) {
        self.st.borrow_mut().watch.toggle();
        super::redraw();
    }

    fn elapsed(&self) -> Duration {
        self.st.borrow().watch.elapsed()
    }

    fn dim(&self) -> f64 {
        if self.st.borrow().watch.is_running() { 1.0 } else { 0.55 }
    }
}

impl Activity for StopwatchActivity {
    fn name(&self) -> String {
        l::t("Stopwatch").to_string()
    }

    fn priority(&self) -> i32 {
        28
    }

    fn kind(&self) -> String {
        "Stopwatch".into()
    }

    fn is_active(&self) -> bool {
        self.st.borrow().active
    }

    fn leading(&self) -> El {
        ui::glyph(icons::TIMER, 15.0, colors::amber())
    }

    fn trailing(&self) -> El {
        ui::digits(ui::format_time(self.elapsed()), 15.0, SEMIBOLD, colors::amber()).opacity(self.dim())
    }

    /// Minimal: the running time, in an oval up to 45 wide.
    fn bubble(&self) -> El {
        let text = ui::format_time(self.elapsed());
        let size = if text.chars().count() > 5 { 10.5 } else { 13.0 };
        ui::digits(text, size, SEMIBOLD, colors::amber()).opacity(self.dim()).halign(Align::Center)
    }

    fn expanded(&self) -> El {
        let t = self.elapsed();
        let paused = !self.st.borrow().watch.is_running();
        let (a, b) = (self.me.clone(), self.me.clone());
        let pause = ui::circle_button(key("stopwatch.pause"), icons::shape(if paused { icons::play() } else { icons::pause() }, 15.0, colors::amber()), colors::amber_dim(), 52.0, move || {
            if let Some(s) = a.upgrade() {
                s.toggle_pause();
            }
        }, Some(l::t("Pause / resume").into()));
        let stop = ui::circle_button(key("stopwatch.stop"), icons::stroke(icons::close(), 14.0, colors::primary(), 2.4), colors::track(), 52.0, move || {
            if let Some(s) = b.upgrade() {
                s.stop();
            }
        }, Some(l::t("Stop").into()))
        .margin(12.0, 0.0, 0.0, 0.0);
        let buttons = El::hstack(vec![pause, stop]).valign(Align::Center);
        let text = format!("{}.{}", ui::format_time(t), t.subsec_millis() / 100);
        let right = El::vstack(vec![
            ui::text(l::t("Stopwatch"), 14.0, SEMIBOLD, colors::amber()).halign(Align::End),
            ui::digits(text, 44.0, LIGHT, colors::amber()).opacity(self.dim()).halign(Align::End).margin(0.0, -6.0, 0.0, -4.0),
        ])
        .align(Align::End, Align::Center);
        El::grid(vec![Len::Auto, Len::Star(1.0)], vec![], vec![(0, 0, buttons), (0, 1, right)]).margin(18.0, 16.0, 22.0, 16.0)
    }

    fn keyline(&self) -> Option<Color> {
        Some(colors::amber())
    }

    fn minimal_width(&self) -> f64 {
        45.0
    }

    fn expanded_size(&self) -> Size {
        Size::new(ALERT_WIDTH, 100.0)
    }
}
