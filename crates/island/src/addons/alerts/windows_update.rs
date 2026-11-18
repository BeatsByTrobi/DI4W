//! "Restart to finish updates" when Windows Update is waiting for a restart. Restart gives a one-minute countdown
//! that can still be cancelled; Later asks again in four hours.

use crate::addons::{Addon, Base, Context};
use crate::model::Alert;
use crate::reg::Key;
use crate::rt::Timer;
use crate::ui::colors;
use crate::ui::el::key;
use crate::ui::{self, ACTION_ALERT_HEIGHT, ALERT_WIDTH, ICON_SIZE};
use di4w_core::{AddonSettings, l};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

pub struct WindowsUpdate {
    base: Base,
    poll: RefCell<Vec<Timer>>,
    snoozed_until: Cell<Option<Instant>>,
    restarting: Rc<Cell<bool>>,
}

/// Windows Update (or servicing) marks a pending restart with these registry keys.
fn restart_pending() -> bool {
    Key::local_machine(r"SOFTWARE\Microsoft\Windows\CurrentVersion\WindowsUpdate\Auto Update\RebootRequired").is_some()
        || Key::local_machine(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Component Based Servicing\RebootPending").is_some()
}

impl WindowsUpdate {
    pub fn new(ctx: &Rc<Context>) -> Rc<WindowsUpdate> {
        Rc::new(WindowsUpdate { base: Base::new(ctx, "WindowsUpdate", "Windows Update"), poll: RefCell::new(Vec::new()), snoozed_until: Cell::new(None), restarting: Rc::new(Cell::new(false)) })
    }

    fn check(self: &Rc<Self>) {
        if self.restarting.get() || self.snoozed_until.get().is_some_and(|t| Instant::now() < t) || !restart_pending() {
            return;
        }
        self.snoozed_until.set(Some(Instant::now() + Duration::from_secs(4 * 3600))); // until the user picks, don't ask again soon
        let me = Rc::downgrade(self);
        self.prompt(Rc::new(move || {
            if let Some(w) = me.upgrade() {
                w.restart();
            }
        }));
    }

    fn prompt(&self, on_restart: Rc<dyn Fn()>) {
        let manager = self.base.ctx.manager.clone();
        let alert = Alert::cyclic(move |weak| {
            Alert::new(move || {
                let (w1, w2, m1, m2, r) = (weak.clone(), weak.clone(), manager.clone(), manager.clone(), on_restart.clone());
                let later = ui::action_button(key("wu.later"), l::t("Later"), move || {
                    if let Some(a) = w1.upgrade() {
                        m1.dismiss_alert(Some(&a));
                    }
                }, None);
                let restart = ui::action_button(key("wu.restart"), l::t("Restart"), move || {
                    if let Some(a) = w2.upgrade() {
                        m2.dismiss_alert(Some(&a));
                    }
                    r();
                }, Some(colors::blue()));
                ui::action_alert_view(ui::glyph_circle("\u{E895}", colors::blue(), ICON_SIZE, 22.0), l::t("Updates are ready"), l::t("Restart to finish installing"), vec![later, restart])
            })
            .size(ALERT_WIDTH, ACTION_ALERT_HEIGHT)
            .keyline(Some(colors::blue()))
            .seconds(15.0)
            .sticky_until_hover(true)
        });
        self.base.show(alert);
    }

    fn restart(&self) {
        self.restarting.set(true);
        crate::interop::run("shutdown.exe", &format!("/r /t 60 /c \"{}\"", l::t("Restarting to finish Windows updates")), None, true);
        let manager = self.base.ctx.manager.clone();
        let restarting = self.restarting.clone();
        let countdown = Alert::cyclic(move |weak| {
            Alert::new(move || {
                let (w, m, r) = (weak.clone(), manager.clone(), restarting.clone());
                let cancel = ui::action_button(key("wu.cancel"), l::t("Cancel"), move || {
                    crate::interop::run("shutdown.exe", "/a", None, true);
                    r.set(false);
                    if let Some(a) = w.upgrade() {
                        m.dismiss_alert(Some(&a));
                    }
                }, Some(colors::red()));
                ui::action_alert_view(ui::glyph_circle("\u{E777}", colors::blue(), ICON_SIZE, 22.0), l::t("Restarting in 1 minute"), l::t("Save your work"), vec![cancel])
            })
            .size(ALERT_WIDTH, ACTION_ALERT_HEIGHT)
            .keyline(Some(colors::red()))
            .sticky()
            .breaks_through(true)
        });
        self.base.show(countdown);
    }
}

impl Addon for WindowsUpdate {
    fn base(&self) -> &Base {
        &self.base
    }

    fn enabled(&self, o: &AddonSettings) -> bool {
        o.windows_update
    }

    fn start(self: Rc<Self>) -> Result<(), String> {
        let (a, b) = (Rc::downgrade(&self), Rc::downgrade(&self));
        *self.poll.borrow_mut() = vec![
            Timer::every(Duration::from_secs(15 * 60), move || {
                if let Some(w) = a.upgrade() {
                    w.check();
                }
            }),
            // First look a little after startup, not while the PC is still busy signing in.
            Timer::once(Duration::from_secs(120), move || {
                if let Some(w) = b.upgrade()
                    && w.base.running() {
                        w.check();
                    }
            }),
        ];
        Ok(())
    }

    fn stop(&self) {
        self.poll.borrow_mut().clear();
    }

    /// The sample from the settings app has the same buttons, but Restart only closes it.
    fn preview(self: Rc<Self>) {
        self.prompt(Rc::new(|| {}));
    }
}
