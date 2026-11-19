//! Warns when the PC stays under heavy load: CPU above 90% for a minute, memory above 90% for 30 s, or (NVIDIA)
//! the GPU above 85 °C. Names the process using the most. At most once per 15 min per kind.

use super::load::{self, CpuMeter};
use crate::addons::{Addon, Base, Context};
use crate::model::{Alert, power};
use crate::rt::Timer;
use crate::ui::colors;
use crate::ui::{self, ICON_SIZE};
use di4w_core::{AddonSettings, l};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

const COOLDOWN: Duration = Duration::from_secs(15 * 60);

pub struct Performance {
    base: Base,
    cpu: RefCell<CpuMeter>,
    since: RefCell<HashMap<&'static str, Instant>>,
    last_warned: RefCell<HashMap<&'static str, Instant>>,
    poll: RefCell<Option<Timer>>,
}

impl Performance {
    pub fn new(ctx: &Rc<Context>) -> Rc<Performance> {
        Rc::new(Performance { base: Base::new(ctx, "PerformanceWarnings", "Performance warnings"), cpu: Default::default(), since: Default::default(), last_warned: Default::default(), poll: Default::default() })
    }

    /// True once the condition has held for the given time (and not warned about recently).
    fn sustained(&self, kind: &'static str, condition: bool, duration: Duration) -> bool {
        if !condition {
            self.since.borrow_mut().remove(kind);
            return false;
        }
        let start = *self.since.borrow_mut().entry(kind).or_insert_with(Instant::now);
        if start.elapsed() < duration {
            return false;
        }
        self.last_warned.borrow().get(kind).is_none_or(|t| t.elapsed() > COOLDOWN)
    }

    fn check(self: &Rc<Self>) {
        let cpu = self.cpu.borrow_mut().read();
        if self.sustained("cpu", cpu > 90.0, Duration::from_secs(60)) {
            self.mark("cpu");
            let me = Rc::downgrade(self);
            crate::rt::spawn(load::top_cpu_process, move |top| {
                if let Some(p) = me.upgrade() {
                    let detail = l::f("{0:0}% for a minute", &[&format!("{cpu:.0}")]) + &top.map(|t| l::f(" · mostly {0}", &[&t])).unwrap_or_default();
                    p.warn(l::t("CPU is working hard").into(), detail);
                }
            });
        }

        let memory = load::memory_percent();
        if self.sustained("memory", memory > 90, Duration::from_secs(30)) {
            self.mark("memory");
            let top = load::top_memory_process();
            let detail = l::f("{0}% in use", &[&memory]) + &top.map(|t| l::f(" · {0} uses the most", &[&t])).unwrap_or_default();
            self.warn(l::t("Memory almost full").into(), detail);
        }

        if let Some(temp) = load::gpu_temperature()
            && self.sustained("gpu", temp >= 85, Duration::from_secs(20)) {
                self.mark("gpu");
                self.warn(l::t("GPU is running hot").into(), format!("{temp} °C"));
            }
    }

    fn mark(&self, kind: &'static str) {
        self.last_warned.borrow_mut().insert(kind, Instant::now());
        self.since.borrow_mut().remove(kind);
    }

    fn warn(&self, title: String, subtitle: String) {
        self.base.show(warning(title, subtitle, true));
    }
}

fn warning(title: String, subtitle: String, open_task_manager: bool) -> Rc<Alert> {
    let mut alert = Alert::new(move || ui::alert_view(ui::glyph_circle("\u{E9D9}", colors::amber(), ICON_SIZE, 22.0), &title, &subtitle, Some(colors::amber()), None))
        .seconds(6.0)
        .sticky_until_hover(true);
    if open_task_manager {
        alert = alert.on_click(|| crate::interop::shell_open("taskmgr.exe"));
    }
    alert.rc()
}

impl Addon for Performance {
    fn base(&self) -> &Base {
        &self.base
    }

    fn enabled(&self, o: &AddonSettings) -> bool {
        o.performance_warnings
    }

    fn start(self: Rc<Self>) -> Result<(), String> {
        self.cpu.borrow_mut().read();
        let weak = Rc::downgrade(&self);
        *self.poll.borrow_mut() = Some(Timer::every(power::interval(Duration::from_secs(5), 3.0), move || {
            if let Some(p) = weak.upgrade() {
                p.check();
            }
        }));
        Ok(())
    }

    fn stop(&self) {
        *self.poll.borrow_mut() = None;
        self.since.borrow_mut().clear();
    }

    fn preview(self: Rc<Self>) {
        self.base.show(warning(l::t("CPU is working hard").into(), l::f("{0:0}% for a minute", &[&96]), false));
    }
}
