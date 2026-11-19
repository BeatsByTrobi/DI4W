//! Brightness bar. Uses the WMI brightness event, which built-in laptop/tablet screens raise. External monitors
//! don't report brightness changes to Windows, so on desktops this stays silent.

use crate::addons::hud::Hud;
use crate::addons::{Addon, Base, Context};
use crate::rt::Handle;
use crate::ui::colors;
use di4w_core::{AddonSettings, l};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoSetProxyBlanket, EOAC_NONE, RPC_C_AUTHN_LEVEL_CALL, RPC_C_IMP_LEVEL_IMPERSONATE};
use windows::Win32::System::Rpc::{RPC_C_AUTHN_WINNT, RPC_C_AUTHZ_NONE};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::System::Wmi::*;
use windows::core::{BSTR, PCWSTR, w};

pub struct Brightness {
    base: Base,
    hud: Hud,
    stop: std::cell::RefCell<Option<Arc<AtomicBool>>>,
}

/// The WMI namespace with the monitor classes.
fn connect() -> windows::core::Result<IWbemServices> {
    unsafe {
        let locator: IWbemLocator = CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER)?;
        let services = locator.ConnectServer(&BSTR::from("root\\wmi"), &BSTR::new(), &BSTR::new(), &BSTR::new(), 0, &BSTR::new(), None)?;
        CoSetProxyBlanket(&services, RPC_C_AUTHN_WINNT, RPC_C_AUTHZ_NONE, PCWSTR::null(), RPC_C_AUTHN_LEVEL_CALL, RPC_C_IMP_LEVEL_IMPERSONATE, None, EOAC_NONE)?;
        Ok(services)
    }
}

fn has_internal_display() -> bool {
    (|| -> windows::core::Result<bool> {
        let services = connect()?;
        unsafe {
            let result = services.ExecQuery(&BSTR::from("WQL"), &BSTR::from("SELECT CurrentBrightness FROM WmiMonitorBrightness"), WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY, None)?;
            let mut objects = [None];
            let mut returned = 0u32;
            let _ = result.Next(WBEM_INFINITE, &mut objects, &mut returned);
            Ok(returned > 0)
        }
    })()
    .unwrap_or(false)
}

impl Brightness {
    pub fn new(ctx: &Rc<Context>) -> Rc<Brightness> {
        Rc::new(Brightness { base: Base::new(ctx, "Brightness", "Brightness"), hud: Hud::new("\u{E706}", colors::primary()), stop: Default::default() })
    }
}

impl Addon for Brightness {
    fn base(&self) -> &Base {
        &self.base
    }

    fn enabled(&self, o: &AddonSettings) -> bool {
        o.brightness
    }

    fn start(self: Rc<Self>) -> Result<(), String> {
        if !has_internal_display() {
            self.base.set_status(Some(l::t("No built-in screen found. External monitors don't report brightness changes to Windows.").into()), false);
            return Ok(()); // nothing to watch; stay "on" but idle
        }
        let weak = Rc::downgrade(&self);
        let events = Handle::new(move |level: u8| {
            if let Some(b) = weak.upgrade() {
                b.hud.set(Some(level as f64 / 100.0), &level.to_string(), None);
                b.base.show(b.hud.alert.clone());
            }
        });
        let stop = Arc::new(AtomicBool::new(false));
        *self.stop.borrow_mut() = Some(stop.clone());
        crate::rt::background(move || {
            let watch = || -> windows::core::Result<()> {
                let services = connect()?;
                let events_query = unsafe {
                    services.ExecNotificationQuery(&BSTR::from("WQL"), &BSTR::from("SELECT * FROM WmiMonitorBrightnessEvent"), WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY, None)?
                };
                while !stop.load(Ordering::SeqCst) {
                    let mut objects = [None];
                    let mut returned = 0u32;
                    let _ = unsafe { events_query.Next(1000, &mut objects, &mut returned) };
                    if returned == 0 {
                        continue;
                    }
                    if let Some(event) = objects[0].take() {
                        let mut value = VARIANT::default();
                        if unsafe { event.Get(w!("Brightness"), 0, &mut value, None, None) }.is_ok() {
                            let level = u32::try_from(&value).or_else(|_| i32::try_from(&value).map(|v| v as u32)).unwrap_or(0);
                            events.send(level.min(100) as u8);
                        }
                    }
                }
                Ok(())
            };
            if let Err(e) = watch() {
                di4w_core::log::error_with("Brightness events unavailable", e);
            }
        });
        Ok(())
    }

    fn stop(&self) {
        if let Some(s) = self.stop.borrow_mut().take() {
            s.store(true, Ordering::SeqCst);
        }
    }

    fn preview(self: Rc<Self>) {
        self.hud.set(Some(0.75), "75", None);
        self.base.show(self.hud.alert.clone());
    }
}
