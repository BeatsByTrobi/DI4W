//! Optional features, each with its own switch in the settings app. The host starts an add-on when its switch is
//! on and stops it when turned off; a failing add-on is logged and left off, it never takes the island down. Their
//! state goes to `status.json` for the settings app ("Set a city to show the weather").

pub mod activity;
pub mod alerts;
pub mod hud;
pub mod info;
pub mod productivity;
pub mod system;

use crate::island::Island;
use crate::model::{Alert, Manager};
use crate::rt::Timer;
use crate::ui::el::El;
use di4w_core::status::{AddonState, IslandStatus};
use di4w_core::{AddonSettings, AppSettings, l};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

/// What an add-on can reach: settings, the alert/activity manager and the island window.
pub struct Context {
    pub settings: Rc<RefCell<AppSettings>>,
    pub manager: Rc<Manager>,
    pub island: Rc<Island>,
    idle_leading: RefCell<Option<Rc<dyn Fn() -> El>>>,
    idle_trailing: RefCell<Option<Rc<dyn Fn() -> El>>>,
    status_changed: RefCell<Option<Rc<dyn Fn()>>>,
}

impl Context {
    pub fn new(settings: Rc<RefCell<AppSettings>>, manager: Rc<Manager>, island: Rc<Island>) -> Rc<Context> {
        Rc::new(Context { settings, manager, island, idle_leading: RefCell::new(None), idle_trailing: RefCell::new(None), status_changed: RefCell::new(None) })
    }

    /// The add-on switches and options.
    pub fn options(&self) -> AddonSettings {
        self.settings.borrow().addons.clone()
    }

    /// Left slot of the empty pill (Do Not Disturb moon).
    pub fn set_idle_leading(&self, view: Option<Rc<dyn Fn() -> El>>) {
        *self.idle_leading.borrow_mut() = view;
        self.island.set_idle_content(self.idle_leading.borrow().clone(), self.idle_trailing.borrow().clone());
    }

    /// Right slot of the empty pill (weather).
    pub fn set_idle_trailing(&self, view: Option<Rc<dyn Fn() -> El>>) {
        *self.idle_trailing.borrow_mut() = view;
        self.island.set_idle_content(self.idle_leading.borrow().clone(), self.idle_trailing.borrow().clone());
    }
}

/// The part every add-on shares: its switch name, whether it runs, and what the settings app shows under it.
pub struct Base {
    pub ctx: Rc<Context>,
    /// The add-on's switch in the settings (e.g. "Clipboard"); also its key in status.json and quiet times.
    pub key: &'static str,
    pub name: &'static str,
    running: Cell<bool>,
    status: RefCell<(Option<String>, bool)>,
}

impl Base {
    pub fn new(ctx: &Rc<Context>, key: &'static str, name: &'static str) -> Base {
        Base { ctx: ctx.clone(), key, name, running: Cell::new(false), status: RefCell::new((None, false)) }
    }

    pub fn running(&self) -> bool {
        self.running.get()
    }

    pub fn options(&self) -> AddonSettings {
        self.ctx.options()
    }

    /// Message for the settings app; `problem` when the user needs to do something.
    pub fn set_status(&self, message: Option<String>, problem: bool) {
        let problem = problem && message.is_some();
        {
            let mut s = self.status.borrow_mut();
            if s.0 == message && s.1 == problem {
                return;
            }
            *s = (message, problem);
        }
        let f = self.ctx.status_changed.borrow().clone();
        if let Some(f) = f {
            f();
        }
    }

    /// Shows a pop-up, unless this add-on's quiet times apply right now (then it only goes to Recent). Previews
    /// from the settings app always show.
    pub fn show(&self, alert: Rc<Alert>) {
        let manager = &self.ctx.manager;
        if manager.recording() {
            let quiet = self.ctx.settings.borrow().addons.quiet.get(self.key).cloned();
            if let Some(q) = quiet.filter(|q| !q.is_empty()) {
                let app = if q.apps.is_empty() { None } else { crate::interop::foreground_process_name() };
                if q.is_quiet(chrono::Local::now().time(), app.as_deref()) {
                    manager.miss(alert);
                    return;
                }
            }
        }
        manager.show_alert(alert);
    }
}

/// An optional feature. Start and stop are only called when the switch changes; `preview` shows a sample of what
/// it pops up, with made-up data (the settings app's Preview button), whether or not it is on.
pub trait Addon {
    fn base(&self) -> &Base;
    fn enabled(&self, options: &AddonSettings) -> bool;
    fn start(self: Rc<Self>) -> Result<(), String>;
    fn stop(&self);
    /// The settings changed while this add-on was running (e.g. a new weather city).
    fn options_changed(self: Rc<Self>) {}
    fn preview(self: Rc<Self>);
}

/// Creates every add-on, starts/stops each one to match its switch, and writes their status.
pub struct Host {
    addons: Vec<Rc<dyn Addon>>,
    ctx: Rc<Context>,
    write_status: RefCell<Option<Timer>>,
    notification_access: RefCell<Option<String>>,
}

impl Host {
    pub fn new(ctx: Rc<Context>) -> Rc<Host> {
        let addons: Vec<Rc<dyn Addon>> = vec![
            system::volume::Volume::new(&ctx),
            system::brightness::Brightness::new(&ctx),
            system::battery::Battery::new(&ctx),
            system::bluetooth::Bluetooth::new(&ctx),
            system::usb::UsbDrives::new(&ctx),
            system::network::Network::new(&ctx),
            system::privacy::Privacy::new(&ctx),
            system::lock_keys::LockKeys::new(&ctx),
            system::performance::Performance::new(&ctx),
            productivity::calendar::Calendar::new(&ctx),
            productivity::clipboard::Clipboard::new(&ctx),
            productivity::screenshots::Screenshots::new(&ctx),
            productivity::downloads::Downloads::new(&ctx),
            productivity::file_copy::FileCopy::new(&ctx),
            alerts::dnd::DoNotDisturb::new(&ctx),
            alerts::windows_update::WindowsUpdate::new(&ctx),
            info::weather::Weather::new(&ctx),
            info::game::GameMode::new(&ctx),
        ];
        let host = Rc::new(Host { addons, ctx: ctx.clone(), write_status: RefCell::new(None), notification_access: RefCell::new(None) });
        let weak = Rc::downgrade(&host);
        *ctx.status_changed.borrow_mut() = Some(Rc::new(move || {
            if let Some(h) = weak.upgrade() {
                h.status_changed();
            }
        }));
        host
    }

    /// Call at startup and whenever the settings change.
    pub fn sync(&self) {
        let options = self.ctx.options();
        for addon in &self.addons {
            Self::sync_one(addon, &options);
        }
        self.status_changed();
    }

    fn sync_one(addon: &Rc<dyn Addon>, options: &AddonSettings) {
        let base = addon.base();
        let wanted = addon.enabled(options);
        if wanted == base.running() {
            if base.running() {
                let a = addon.clone();
                crate::rt::guard(move || a.options_changed());
            }
            return;
        }
        if !wanted {
            base.set_status(None, false);
        }
        let a = addon.clone();
        let result = crate::rt::guard(move || if wanted { a.start() } else {
            a.stop();
            Ok(())
        })
        .unwrap_or_else(|| Err("unexpected error".to_string()));
        match result {
            Ok(()) => {
                base.running.set(wanted);
                di4w_core::log::info(format!("Add-on {} {}", base.name, if wanted { "on" } else { "off" }));
            }
            Err(e) => {
                di4w_core::log::error(format!("Add-on {} failed to {}: {e}", base.name, if wanted { "start" } else { "stop" }));
                if wanted {
                    let a = addon.clone();
                    crate::rt::guard(move || a.stop());
                    base.set_status(Some(l::f("Couldn't start: {0}", &[&e])), true);
                }
                base.running.set(false);
            }
        }
    }

    pub fn shutdown(&self) {
        for addon in &self.addons {
            if addon.base().running() {
                let a = addon.clone();
                crate::rt::guard(move || a.stop());
                addon.base().running.set(false);
            }
        }
    }

    /// A sample pop-up for the add-on with this key; shown even during Do Not Disturb, since it was asked for.
    pub fn preview(&self, key: &str) {
        let Some(addon) = self.addons.iter().find(|a| a.base().key == key).cloned() else { return };
        let manager = &self.ctx.manager;
        let quiet = manager.quiet();
        manager.set_recording(false);
        manager.set_quiet(false);
        let name = addon.base().name;
        if crate::rt::guard(move || addon.preview()).is_none() {
            di4w_core::log::error(format!("Preview of {name} failed"));
        }
        manager.set_quiet(quiet);
        manager.set_recording(true);
    }

    /// Windows notification access, shown on the Notifications page when it isn't allowed.
    pub fn set_notification_access(&self, access: String) {
        *self.notification_access.borrow_mut() = Some(access);
        self.status_changed();
    }

    /// Coalesces bursts (startup, several add-ons changing at once) into one write.
    fn status_changed(&self) {
        let addons: Vec<Rc<dyn Addon>> = self.addons.clone();
        let access = self.notification_access.clone();
        *self.write_status.borrow_mut() = Some(Timer::once(Duration::from_millis(500), move || {
            let status = IslandStatus {
                addons: addons
                    .iter()
                    .map(|a| {
                        let b = a.base();
                        let s = b.status.borrow();
                        (b.key.to_string(), AddonState { running: b.running(), message: s.0.clone(), problem: s.1 })
                    })
                    .collect(),
                notification_access: access.borrow().clone(),
            };
            status.save();
        }));
    }
}
