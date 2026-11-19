//! Charger plugged in / unplugged with the charge level, and warnings when the battery runs low.

use crate::addons::{Addon, Base, Context};
use crate::model::Alert;
use crate::rt::Handle;
use crate::ui::colors::{self, Color};
use crate::ui::el::El;
use crate::ui::{self, ICON_SIZE, INDICATOR_HEIGHT, INDICATOR_WIDTH, SEMIBOLD, icons};
use di4w_core::{AddonSettings, l};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use windows::Foundation::EventHandler;
use windows::System::Power::{BatteryStatus, PowerManager, PowerSupplyStatus};

pub struct Battery {
    base: Base,
    plugged_in: Cell<bool>,
    last_percent: Cell<i32>,
    tokens: RefCell<Option<(i64, i64, Handle<()>)>>,
}

fn plugged_in() -> bool {
    PowerManager::PowerSupplyStatus().is_ok_and(|s| s != PowerSupplyStatus::NotPresent)
}

fn percent() -> i32 {
    PowerManager::RemainingChargePercent().unwrap_or(100)
}

impl Battery {
    pub fn new(ctx: &Rc<Context>) -> Rc<Battery> {
        Rc::new(Battery { base: Base::new(ctx, "Battery", "Battery"), plugged_in: Cell::new(false), last_percent: Cell::new(100), tokens: RefCell::new(None) })
    }

    fn changed(&self) {
        let plugged = plugged_in();
        if plugged != self.plugged_in.get() {
            self.plugged_in.set(plugged);
            self.indicate(percent(), plugged);
        }

        let percent = percent();
        let previous = self.last_percent.replace(percent);
        if self.plugged_in.get() || percent >= previous {
            return;
        }
        // Warn once when crossing each threshold on the way down.
        let o = self.base.options();
        if previous > o.battery_critical_percent && percent <= o.battery_critical_percent {
            self.announce(l::t("Battery very low"), &l::f("{0}% left. Plug in your charger.", &[&percent]), percent, colors::red(), true);
        } else if previous > o.battery_low_percent && percent <= o.battery_low_percent {
            self.announce(l::t("Low battery"), &l::f("{0}% left", &[&percent]), percent, colors::amber(), false);
        }
    }

    /// The iPhone's charging indicator: the island widens with "Charging" on the leading side and the percentage
    /// with a battery on the trailing side, green while charging (red when low).
    fn indicate(&self, percent: i32, charging: bool) {
        let color = if percent <= self.base.options().battery_low_percent {
            colors::red()
        } else if charging {
            colors::green()
        } else {
            colors::primary()
        };
        let neutral = color == colors::primary();
        let alert = Alert::new(move || {
            let label = ui::text(if charging { l::t("Charging") } else { l::t("On battery") }, 15.0, SEMIBOLD, colors::primary());
            let level = ui::digits(format!("{percent}%"), 15.0, SEMIBOLD, color).margin(0.0, 0.0, 7.0, 0.0);
            let battery = El::hstack(vec![level, icons::battery(percent as f64 / 100.0, charging, color, 27.0)]);
            ui::indicator(label, battery)
        })
        .size(INDICATOR_WIDTH, INDICATOR_HEIGHT)
        .seconds(2.5)
        .keyline(if neutral { None } else { Some(color) });
        self.base.show(alert.rc());
    }

    fn announce(&self, title: &str, subtitle: &str, percent: i32, color: Color, urgent: bool) {
        let (title, subtitle) = (title.to_string(), subtitle.to_string());
        let alert = Alert::new(move || {
            let icon = El::boxed(vec![icons::battery(percent as f64 / 100.0, false, color, 34.0)]).size(ICON_SIZE, ICON_SIZE);
            ui::alert_view(icon, &title, &subtitle, Some(color), None)
        })
        .seconds(if urgent { 8.0 } else { 3.5 })
        .sticky_until_hover(urgent)
        .breaks_through(urgent)
        .keyline(Some(color));
        self.base.show(alert.rc());
    }
}

impl Addon for Battery {
    fn base(&self) -> &Base {
        &self.base
    }

    fn enabled(&self, o: &AddonSettings) -> bool {
        o.battery
    }

    fn start(self: Rc<Self>) -> Result<(), String> {
        if PowerManager::BatteryStatus().is_ok_and(|s| s == BatteryStatus::NotPresent) {
            self.base.set_status(Some(l::t("This PC has no battery.").into()), false);
            return Ok(()); // desktop PC: nothing to show
        }
        self.plugged_in.set(plugged_in());
        self.last_percent.set(percent());
        let weak = Rc::downgrade(&self);
        let handle = Handle::new(move |()| {
            if let Some(b) = weak.upgrade() {
                b.changed();
            }
        });
        let (h1, h2) = (handle.clone(), handle.clone());
        let a = PowerManager::PowerSupplyStatusChanged(&EventHandler::new(move |_, _| {
            h1.send(());
            Ok(())
        }))
        .map_err(|e| e.message())?;
        let b = PowerManager::RemainingChargePercentChanged(&EventHandler::new(move |_, _| {
            h2.send(());
            Ok(())
        }))
        .map_err(|e| e.message())?;
        *self.tokens.borrow_mut() = Some((a, b, handle));
        Ok(())
    }

    fn stop(&self) {
        if let Some((a, b, h)) = self.tokens.borrow_mut().take() {
            let _ = PowerManager::RemovePowerSupplyStatusChanged(a);
            let _ = PowerManager::RemoveRemainingChargePercentChanged(b);
            h.close();
        }
    }

    fn preview(self: Rc<Self>) {
        self.indicate(82, true);
    }
}
