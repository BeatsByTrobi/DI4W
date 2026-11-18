//! Windows' battery saver (energy saver). While it's on the island does less: the music bars redraw 10 times a
//! second instead of 30, blur and shadow effects are skipped, and background checks run less often.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use windows::Foundation::EventHandler;
use windows::System::Power::{EnergySaverStatus, PowerManager};

static SAVING: AtomicBool = AtomicBool::new(false);
static STARTED: AtomicBool = AtomicBool::new(false);

thread_local! {
    static LISTENERS: RefCell<Vec<Rc<dyn Fn()>>> = const { RefCell::new(Vec::new()) };
}

pub fn saving() -> bool {
    SAVING.load(Ordering::Relaxed)
}

/// Calls `f` on the UI thread when battery saver turns on or off.
pub fn on_change(f: impl Fn() + 'static) {
    LISTENERS.with(|l| l.borrow_mut().push(Rc::new(f)));
}

/// An interval that stretches while battery saver is on.
pub fn interval(normal: Duration, saving_factor: f64) -> Duration {
    if saving() { normal.mul_f64(saving_factor) } else { normal }
}

fn read() -> bool {
    PowerManager::EnergySaverStatus().is_ok_and(|s| s == EnergySaverStatus::On)
}

pub fn start() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    SAVING.store(read(), Ordering::Relaxed);
    let _ = PowerManager::EnergySaverStatusChanged(&EventHandler::new(|_, _| {
        crate::rt::post(|| {
            let saving = read();
            if saving == SAVING.load(Ordering::Relaxed) {
                return;
            }
            SAVING.store(saving, Ordering::Relaxed);
            di4w_core::log::info(format!("Battery saver {}", if saving { "on: reducing effects and checks" } else { "off" }));
            let listeners = LISTENERS.with(|l| l.borrow().clone());
            for f in listeners {
                f();
            }
        });
        Ok(())
    }));
}
