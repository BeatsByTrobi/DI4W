//! Progress of File Explorer copies and moves. Explorer's progress dialog ("OperationStatusWindow") puts the
//! percentage in its title, in any language ("23% complete", "23% voltooid"), even when minimized.

use crate::addons::activity::AddonActivity;
use crate::addons::{Addon, Base, Context};
use crate::model::{Alert, power};
use crate::rt::Timer;
use crate::ui::colors;
use crate::ui::el::{Align, El};
use crate::ui::{self, ALERT_WIDTH, ICON_SIZE, SEMIBOLD};
use di4w_core::{AddonSettings, l};
use regex::Regex;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::Duration;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::FindWindowExW;
use windows::core::w;

const BAR_WIDTH: f64 = ALERT_WIDTH - 2.0 * 18.0; // expanded width minus margins
const GLYPH: &str = "\u{E8C8}";

fn idle_interval() -> Duration {
    power::interval(Duration::from_secs(3), 2.0)
}

/// The lowest percentage across open copy dialogs (the one that will finish last), with its title.
fn read_progress() -> Option<(i32, String)> {
    static PERCENT: OnceLock<Regex> = OnceLock::new();
    let pattern = PERCENT.get_or_init(|| Regex::new(r"(\d{1,3})\s?%").unwrap());
    let mut lowest: Option<(i32, String)> = None;
    let mut hwnd = HWND::default();
    loop {
        hwnd = unsafe { FindWindowExW(None, Some(hwnd), w!("OperationStatusWindow"), None) }.unwrap_or_default();
        if hwnd.is_invalid() {
            break;
        }
        let title = crate::interop::window_text(hwnd);
        let Some(value) = pattern.captures(&title).and_then(|c| c[1].parse::<i32>().ok()) else { continue };
        if lowest.as_ref().is_none_or(|(l, _)| value < *l) {
            lowest = Some((value, title));
        }
    }
    lowest
}

pub struct FileCopy {
    base: Base,
    state: Rc<RefCell<(i32, String)>>,
    activity: Rc<AddonActivity>,
    poll: RefCell<Option<Timer>>,
    last_percent: Cell<i32>,
}

impl FileCopy {
    pub fn new(ctx: &Rc<Context>) -> Rc<FileCopy> {
        let state = Rc::new(RefCell::new((0, String::new())));
        let (a, b) = (state.clone(), state.clone());
        let activity = AddonActivity::new(
            ctx.manager.clone(),
            "File copy",
            "FileCopy",
            12,
            Box::new(|| ui::glyph(GLYPH, 15.0, colors::teal())),
            Box::new(move || ui::digits(format!("{}%", a.borrow().0), 14.0, SEMIBOLD, colors::teal())),
            Box::new(|| ui::glyph(GLYPH, 13.0, colors::teal())),
            Box::new(move || {
                let (percent, title) = b.borrow().clone();
                let fill = El::empty().w((BAR_WIDTH * percent as f64 / 100.0).max(6.0)).h(6.0).radius(3.0).bg(colors::teal()).halign(Align::Start);
                let track = El::boxed(vec![fill]).size(BAR_WIDTH, 6.0).radius(3.0).bg(colors::track()).margin(0.0, 12.0, 0.0, 0.0).halign(Align::Start);
                El::vstack(vec![ui::text(title, 15.0, SEMIBOLD, colors::primary()), track]).margin(18.0, 18.0, 18.0, 16.0)
            }),
            230.0,
            84.0,
            Some(colors::teal()),
        );
        Rc::new(FileCopy { base: Base::new(ctx, "FileCopy", "File copy"), state, activity, poll: RefCell::new(None), last_percent: Cell::new(-1) })
    }

    fn check(&self) {
        match read_progress() {
            None => {
                // The dialog closed. Explorer doesn't say whether it finished or was cancelled; near the end means done.
                if self.last_percent.get() >= 90 {
                    self.announce_done();
                }
                self.last_percent.set(-1);
                self.activity.hide();
                if let Some(p) = self.poll.borrow().as_ref() {
                    p.set_interval(idle_interval()); // nothing copying: look less often
                }
            }
            Some((percent, title)) => {
                self.last_percent.set(percent);
                *self.state.borrow_mut() = (percent, title);
                self.activity.show();
                if let Some(p) = self.poll.borrow().as_ref() {
                    p.set_interval(Duration::from_secs(1)); // copying: keep the percentage live
                }
            }
        }
    }

    fn announce_done(&self) {
        let alert = Alert::new(|| ui::alert_view(ui::glyph_circle(GLYPH, colors::teal(), ICON_SIZE, 22.0), l::t("Files copied"), l::t("File Explorer finished"), None, None)).seconds(3.0);
        self.base.show(alert.rc());
    }
}

impl Addon for FileCopy {
    fn base(&self) -> &Base {
        &self.base
    }

    fn enabled(&self, o: &AddonSettings) -> bool {
        o.file_copy
    }

    fn start(self: Rc<Self>) -> Result<(), String> {
        let weak = Rc::downgrade(&self);
        *self.poll.borrow_mut() = Some(Timer::every(idle_interval(), move || {
            if let Some(f) = weak.upgrade() {
                f.check();
            }
        }));
        Ok(())
    }

    fn stop(&self) {
        *self.poll.borrow_mut() = None;
        self.activity.hide();
        self.last_percent.set(-1);
    }

    fn preview(self: Rc<Self>) {
        self.announce_done();
    }
}
