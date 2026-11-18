//! Live activities and alerts, the two things the island shows.

use crate::ui::colors::Color;
use crate::ui::el::El;
use crate::ui::geom::Size;
use std::cell::Cell;
use std::rc::{Rc, Weak};
use std::time::Duration;

/// A live activity shown in the island, the Windows equivalent of an iOS Live Activity.
/// It supplies the compact leading/trailing views, the small "minimal" view used when two activities run at
/// once (attached to the island's leading side, or in the detached bubble), and the expanded view. Views are built
/// from the activity's current state each time the island draws.
pub trait Activity {
    fn name(&self) -> String;

    /// Extra words for screen readers when the compact view has no text of its own (e.g. the song).
    fn accessible_detail(&self) -> Option<String> {
        None
    }

    /// Higher wins the main pill; the runner-up gets the bubble (unless the user set an order).
    fn priority(&self) -> i32;

    /// Stable id for the user's order in the settings ("Timer", "Media"…), not translated.
    fn kind(&self) -> String {
        self.name()
    }

    /// The mouse moved onto or off the compact pill while this activity is in it.
    fn on_hover(&self, _hovering: bool) {}

    fn is_active(&self) -> bool;

    fn leading(&self) -> El;
    fn trailing(&self) -> El;
    fn bubble(&self) -> El;
    fn expanded(&self) -> El;

    /// Tint of the hairline around the island while this activity shows (Apple's key line); `None` is neutral.
    fn keyline(&self) -> Option<Color> {
        None
    }

    fn compact_width(&self) -> f64 {
        230.0
    }

    /// Width of the minimal view's spot: a 37 circle, up to a 45 wide oval for text such as a countdown.
    fn minimal_width(&self) -> f64 {
        37.0
    }

    fn expanded_size(&self) -> Size {
        Size::new(371.0, 160.0)
    }
}

/// A transient pop-out (notification, download complete, timer finished).
pub struct Alert {
    /// Builds the view (each time it's drawn, so it can change, like the volume bar).
    pub content: Box<dyn Fn() -> El>,
    pub size: Size,
    /// Tint of the hairline around the island while this alert shows; `None` is neutral.
    pub keyline: Option<Color>,
    pub duration: Duration,
    /// Never auto-dismisses; must be clicked / dismissed in code.
    pub sticky: Cell<bool>,
    /// Stays until the mouse has hovered it once, then times out normally.
    pub sticky_until_hover: Cell<bool>,
    /// Shown right away, putting any current alert back in line (volume/brightness bars).
    pub urgent: bool,
    /// Still shown while Do Not Disturb is on (alarms, timers, calls).
    pub breaks_through_quiet: bool,
    pub on_click: Option<Rc<dyn Fn()>>,
    pub on_dismissed: Option<Rc<dyn Fn()>>,
}

impl Alert {
    /// An alert with the standard 371 × 84 size and 4.5 s duration.
    pub fn new(content: impl Fn() -> El + 'static) -> Alert {
        Alert {
            content: Box::new(content),
            size: Size::new(371.0, 84.0),
            keyline: None,
            duration: Duration::from_secs_f64(4.5),
            sticky: Cell::new(false),
            sticky_until_hover: Cell::new(false),
            urgent: false,
            breaks_through_quiet: false,
            on_click: None,
            on_dismissed: None,
        }
    }

    /// An alert whose view (or callbacks) need the alert itself, e.g. a button that dismisses it.
    pub fn cyclic(build: impl FnOnce(Weak<Alert>) -> Alert) -> Rc<Alert> {
        Rc::new_cyclic(|weak| build(weak.clone()))
    }

    pub fn size(mut self, w: f64, h: f64) -> Alert {
        self.size = Size::new(w, h);
        self
    }

    pub fn height(mut self, h: f64) -> Alert {
        self.size.h = h;
        self
    }

    pub fn keyline(mut self, color: Option<Color>) -> Alert {
        self.keyline = color;
        self
    }

    pub fn seconds(mut self, s: f64) -> Alert {
        self.duration = Duration::from_secs_f64(s);
        self
    }

    pub fn sticky(self) -> Alert {
        self.sticky.set(true);
        self
    }

    pub fn sticky_until_hover(self, on: bool) -> Alert {
        self.sticky_until_hover.set(on);
        self
    }

    pub fn urgent(mut self) -> Alert {
        self.urgent = true;
        self
    }

    pub fn breaks_through(mut self, on: bool) -> Alert {
        self.breaks_through_quiet = on;
        self
    }

    pub fn on_click(mut self, f: impl Fn() + 'static) -> Alert {
        self.on_click = Some(Rc::new(f));
        self
    }

    pub fn on_click_opt(mut self, f: Option<Rc<dyn Fn()>>) -> Alert {
        self.on_click = f;
        self
    }

    pub fn on_dismissed(mut self, f: impl Fn() + 'static) -> Alert {
        self.on_dismissed = Some(Rc::new(f));
        self
    }

    pub fn rc(self) -> Rc<Alert> {
        Rc::new(self)
    }
}
