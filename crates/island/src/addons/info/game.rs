//! While a fullscreen game runs, keep the island visible and show GPU load and temperature (CPU and RAM when
//! expanded). A fullscreen app counts as a game when its GPU 3D load is high, so videos still hide it. Frame rate
//! isn't shown: Windows only exposes it to programs running as administrator.

use crate::addons::activity::AddonActivity;
use crate::addons::system::load::{self, CpuMeter};
use crate::addons::{Addon, Base, Context};
use crate::rt::Timer;
use crate::ui::colors;
use crate::ui::el::{Align, El, Len};
use crate::ui::{self, LIGHT, SEMIBOLD};
use di4w_core::{AddonSettings, l};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

const GLYPH: &str = "\u{E7FC}";

#[derive(Clone)]
struct Stats {
    compact: String,
    gpu: String,
    temp: String,
    cpu: String,
    ram: String,
}

pub struct GameMode {
    base: Base,
    stats: Rc<RefCell<Stats>>,
    activity: Rc<AddonActivity>,
    cpu: RefCell<CpuMeter>,
    poll: RefCell<Option<Timer>>,
    preview_done: RefCell<Option<Timer>>,
    game_ticks: Cell<u32>,
}

fn stat(label: &str, value: &str) -> El {
    El::vstack(vec![ui::text(label, 12.0, SEMIBOLD, colors::green()).halign(Align::Center), ui::digits(value, 22.0, LIGHT, colors::primary()).halign(Align::Center)]).halign(Align::Center)
}

impl GameMode {
    pub fn new(ctx: &Rc<Context>) -> Rc<GameMode> {
        let dash = "\u{2013}".to_string();
        let stats = Rc::new(RefCell::new(Stats { compact: String::new(), gpu: dash.clone(), temp: dash.clone(), cpu: dash.clone(), ram: dash }));
        let (a, b) = (stats.clone(), stats.clone());
        let activity = AddonActivity::new(
            ctx.manager.clone(),
            "Game",
            "Game",
            10,
            Box::new(|| ui::glyph(GLYPH, 16.0, colors::green())),
            Box::new(move || ui::digits(a.borrow().compact.as_str(), 14.0, SEMIBOLD, colors::green())),
            Box::new(|| ui::glyph(GLYPH, 13.0, colors::green())),
            Box::new(move || {
                let s = b.borrow().clone();
                El::grid(
                    vec![Len::Star(1.0); 4],
                    vec![],
                    vec![(0, 0, stat("GPU", &s.gpu)), (0, 1, stat(l::t("GPU temp"), &s.temp)), (0, 2, stat("CPU", &s.cpu)), (0, 3, stat(l::t("Memory"), &s.ram))],
                )
                .margin(22.0, 16.0, 22.0, 14.0)
            }),
            250.0,
            88.0,
            Some(colors::green()),
        );
        Rc::new(GameMode {
            base: Base::new(ctx, "GameMode", "Game mode"),
            stats,
            activity,
            cpu: RefCell::new(CpuMeter::default()),
            poll: RefCell::new(None),
            preview_done: RefCell::new(None),
            game_ticks: Cell::new(0),
        })
    }

    fn check(&self) {
        let gpu = load::gpu_percent().unwrap_or(0.0);
        let cpu = self.cpu.borrow_mut().read();
        let fullscreen = crate::interop::is_fullscreen_app_running();
        // Two readings in a row, so a short spike (or a fullscreen video) doesn't count as a game.
        self.game_ticks.set(if fullscreen && gpu >= 30.0 { self.game_ticks.get() + 1 } else { 0 });
        let in_game = self.game_ticks.get() >= 2 || (self.activity_active() && fullscreen);
        if !in_game {
            self.leave();
            return;
        }
        let temp = load::gpu_temperature();
        *self.stats.borrow_mut() = Stats {
            compact: match temp {
                Some(t) => format!("GPU {gpu:.0}% · {t}°"),
                None => format!("GPU {gpu:.0}%"),
            },
            gpu: format!("{gpu:.0}%"),
            temp: temp.map_or("\u{2013}".into(), |t| format!("{t}°C")),
            cpu: format!("{cpu:.0}%"),
            ram: format!("{}%", load::memory_percent()),
        };
        self.base.ctx.island.keep_visible_in_fullscreen.set(true);
        self.activity.show();
    }

    fn activity_active(&self) -> bool {
        use crate::model::Activity;
        self.activity.is_active()
    }

    fn leave(&self) {
        self.game_ticks.set(0);
        self.base.ctx.island.keep_visible_in_fullscreen.set(false);
        self.activity.hide();
    }
}

impl Addon for GameMode {
    fn base(&self) -> &Base {
        &self.base
    }

    fn enabled(&self, o: &AddonSettings) -> bool {
        o.game_mode
    }

    fn start(self: Rc<Self>) -> Result<(), String> {
        self.cpu.borrow_mut().read();
        if load::gpu_temperature().is_none() {
            self.base.set_status(Some(l::t("GPU temperature needs an NVIDIA graphics card; load is still shown.").into()), false);
        }
        let weak = Rc::downgrade(&self);
        *self.poll.borrow_mut() = Some(Timer::every(Duration::from_secs(2), move || {
            if let Some(g) = weak.upgrade() {
                g.check();
            }
        }));
        Ok(())
    }

    fn stop(&self) {
        *self.poll.borrow_mut() = None;
        self.leave();
    }

    /// The game view with sample numbers for a few seconds (unless a real game shows it meanwhile).
    fn preview(self: Rc<Self>) {
        if self.game_ticks.get() >= 2 {
            return; // the real thing is on screen
        }
        *self.stats.borrow_mut() = Stats { compact: "GPU 72% · 64°".into(), gpu: "72%".into(), temp: "64°C".into(), cpu: "35%".into(), ram: "58%".into() };
        self.activity.show();
        let weak = Rc::downgrade(&self);
        *self.preview_done.borrow_mut() = Some(Timer::once(Duration::from_secs(6), move || {
            if let Some(g) = weak.upgrade()
                && g.game_ticks.get() < 2 {
                    g.activity.hide();
                }
        }));
    }
}
