//! Follows Windows' Do Not Disturb / Focus: the island goes quiet (only alarms, timers and urgent alerts) and
//! shows a moon in the empty pill.

use crate::addons::hud::Hud;
use crate::addons::{Addon, Base, Context};
use crate::model::power;
use crate::rt::Timer;
use crate::ui::el::El;
use crate::ui::{self, colors, icons};
use di4w_core::{AddonSettings, l};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::UI::Shell::{QUNS_QUIET_TIME, SHQueryUserNotificationState};
use windows::core::{s, w};

// Windows notification state "quiet hours profile" (what Do Not Disturb / Focus assist sets): 0 = off.
const WNF_SHEL_QUIETHOURS_ACTIVE_PROFILE_CHANGED: u64 = 0x0D83_063E_A3BF_1C75;

type QueryWnf = unsafe extern "system" fn(*const u64, *const core::ffi::c_void, *const core::ffi::c_void, *mut u32, *mut i32, *mut u32) -> i32;

fn is_quiet() -> bool {
    if unsafe { SHQueryUserNotificationState() }.is_ok_and(|s| s == QUNS_QUIET_TIME) {
        return true;
    }
    unsafe {
        let Ok(ntdll) = GetModuleHandleW(w!("ntdll.dll")) else { return false };
        let Some(proc) = GetProcAddress(ntdll, s!("NtQueryWnfStateData")) else { return false };
        let query: QueryWnf = std::mem::transmute(proc);
        let name = WNF_SHEL_QUIETHOURS_ACTIVE_PROFILE_CHANGED;
        let (mut stamp, mut profile, mut size) = (0u32, 0i32, 4u32);
        query(&name, std::ptr::null(), std::ptr::null(), &mut stamp, &mut profile, &mut size) == 0 && size >= 4 && profile != 0
    }
}

pub struct DoNotDisturb {
    base: Base,
    hud: Hud,
    moon: Rc<dyn Fn() -> El>,
    poll: RefCell<Option<Timer>>,
    quiet: Cell<bool>,
}

impl DoNotDisturb {
    pub fn new(ctx: &Rc<Context>) -> Rc<DoNotDisturb> {
        Rc::new(DoNotDisturb {
            base: Base::new(ctx, "DoNotDisturb", "Do Not Disturb"),
            hud: Hud::new(icons::MOON, colors::indigo()),
            moon: Rc::new(|| ui::glyph(icons::MOON, 14.0, colors::indigo()).tooltip(l::t("Do Not Disturb is on"))),
            poll: RefCell::new(None),
            quiet: Cell::new(false),
        })
    }

    /// `announce`: show the change in the island, as the iPhone does when a Focus turns on or off.
    fn apply(&self, quiet: bool, announce: bool) {
        if quiet == self.quiet.get() {
            return;
        }
        self.quiet.set(quiet);
        if announce {
            // Before going quiet, or it would be held back itself.
            self.hud.set(None, if quiet { l::t("Do Not Disturb is on") } else { l::t("Do Not Disturb is off") }, None);
            self.base.show(self.hud.alert.clone());
        }
        self.base.ctx.manager.set_quiet(quiet);
        self.base.ctx.set_idle_leading(if quiet { Some(self.moon.clone()) } else { None });
    }
}

impl Addon for DoNotDisturb {
    fn base(&self) -> &Base {
        &self.base
    }

    fn enabled(&self, o: &AddonSettings) -> bool {
        o.do_not_disturb
    }

    fn start(self: Rc<Self>) -> Result<(), String> {
        let weak = Rc::downgrade(&self);
        *self.poll.borrow_mut() = Some(Timer::every(power::interval(Duration::from_secs(3), 3.0), move || {
            if let Some(d) = weak.upgrade() {
                d.apply(is_quiet(), true);
            }
        }));
        self.apply(is_quiet(), false);
        Ok(())
    }

    fn stop(&self) {
        *self.poll.borrow_mut() = None;
        self.apply(false, false);
    }

    /// The real thing is just the moon in the empty island; the sample says what it means.
    fn preview(self: Rc<Self>) {
        self.hud.set(None, l::t("Do Not Disturb is on"), None);
        self.base.show(self.hud.alert.clone());
    }
}
