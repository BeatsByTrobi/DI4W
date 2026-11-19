//! A system indicator the way the iPhone shows one (Silent mode, Focus, charging): the island just widens,
//! staying one line tall, with an icon on the leading side and a level bar or a short text on the trailing side.
//! One view and one alert are reused, so holding a key updates the bar in place instead of queuing pop-ups.

use crate::model::Alert;
use crate::ui::colors::{self, Color};
use crate::ui::el::{Align, El, TextAlign};
use crate::ui::{self, INDICATOR_HEIGHT, INDICATOR_WIDTH, SEMIBOLD};
use std::cell::RefCell;
use std::rc::Rc;

const BAR_WIDTH: f64 = 170.0;

struct State {
    glyph: String,
    level: Option<f64>,
    value: String,
}

pub struct Hud {
    state: Rc<RefCell<State>>,
    pub alert: Rc<Alert>,
}

impl Hud {
    pub fn new(glyph: &str, accent: Color) -> Hud {
        let state = Rc::new(RefCell::new(State { glyph: glyph.into(), level: None, value: String::new() }));
        let s = state.clone();
        let neutral = accent == colors::primary();
        let alert = Alert::new(move || {
            let s = s.borrow();
            let icon = ui::glyph(&s.glyph, 17.0, accent);
            let trailing = match s.level {
                // The bar has no words of its own; screen readers and the history read the value from the tooltip.
                Some(level) => {
                    let fill = El::empty().w(BAR_WIDTH * level.clamp(0.0, 1.0)).h(6.0).radius(3.0).bg(colors::primary()).halign(Align::Start);
                    El::boxed(vec![fill]).size(BAR_WIDTH, 6.0).radius(3.0).bg(colors::track()).tooltip(s.value.clone())
                }
                None => ui::with_text(ui::text(s.value.as_str(), 15.0, SEMIBOLD, if neutral { colors::primary() } else { accent }), |t| t.align = TextAlign::Right),
            };
            ui::indicator(icon, El::boxed(vec![trailing]))
        })
        .size(INDICATOR_WIDTH, INDICATOR_HEIGHT)
        .seconds(1.6)
        .urgent()
        .keyline(if neutral { None } else { Some(accent) });
        Hud { state, alert: alert.rc() }
    }

    /// Level 0..1 as a bar (volume, brightness), or no level and a short text ("Caps Lock on").
    pub fn set(&self, level: Option<f64>, value: &str, glyph: Option<&str>) {
        let mut s = self.state.borrow_mut();
        if let Some(g) = glyph {
            s.glyph = g.into();
        }
        s.level = level;
        s.value = value.into();
        drop(s);
        crate::activities::redraw();
    }
}
