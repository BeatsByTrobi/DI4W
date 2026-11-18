//! A live activity driven by an add-on or the Island API (download in progress, file copy, game overlay, script
//! progress): its views are closures over the add-on's own state.

use crate::model::{Activity, Manager};
use crate::ui::colors::Color;
use crate::ui::el::El;
use crate::ui::geom::Size;
use crate::ui::ALERT_WIDTH;
use std::cell::Cell;
use std::rc::Rc;

pub type View = Box<dyn Fn() -> El>;

pub struct AddonActivity {
    name: String,
    kind: String,
    priority: i32,
    compact_width: f64,
    expanded_size: Size,
    /// The key line's tint while this activity shows.
    pub tint: Option<Color>,
    active: Cell<bool>,
    manager: Rc<Manager>,
    leading: View,
    trailing: View,
    bubble: View,
    expanded: View,
}

impl AddonActivity {
    #[allow(clippy::too_many_arguments)]
    pub fn new(manager: Rc<Manager>, name: &str, kind: &str, priority: i32, leading: View, trailing: View, bubble: View, expanded: View, compact_width: f64, expanded_height: f64, tint: Option<Color>) -> Rc<AddonActivity> {
        let activity = Rc::new(AddonActivity {
            name: name.into(),
            kind: kind.into(),
            priority,
            compact_width,
            expanded_size: Size::new(ALERT_WIDTH, expanded_height),
            tint,
            active: Cell::new(false),
            manager: manager.clone(),
            leading,
            trailing,
            bubble,
            expanded,
        });
        manager.register(activity.clone());
        activity
    }

    pub fn show(&self) {
        if !self.active.replace(true) {
            self.manager.changed();
        } else {
            crate::activities::redraw();
        }
    }

    pub fn hide(&self) {
        if self.active.replace(false) {
            self.manager.changed();
        }
    }
}

impl Activity for AddonActivity {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn priority(&self) -> i32 {
        self.priority
    }

    fn kind(&self) -> String {
        self.kind.clone()
    }

    fn is_active(&self) -> bool {
        self.active.get()
    }

    fn leading(&self) -> El {
        (self.leading)()
    }

    fn trailing(&self) -> El {
        (self.trailing)()
    }

    fn bubble(&self) -> El {
        (self.bubble)()
    }

    fn expanded(&self) -> El {
        (self.expanded)()
    }

    fn keyline(&self) -> Option<Color> {
        self.tint
    }

    fn compact_width(&self) -> f64 {
        self.compact_width
    }

    fn expanded_size(&self) -> Size {
        self.expanded_size
    }
}
