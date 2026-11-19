//! The Recent tab of the panel: the last alerts, newest first, including ones Do Not Disturb held back (marked
//! with a moon). Click one to do what the alert did (open the chat, the drive, the download…).

use crate::model::manager::PastAlert;
use crate::ui::colors;
use crate::ui::el::{Align, Cursor, El, Kind, Len, Run};
use crate::ui::{self, MEDIUM, NORMAL, SEMIBOLD, icons};
use di4w_core::l;
use std::cell::Cell;
use std::rc::Rc;

thread_local! {
    /// The list's scroll position, kept while the panel is open (views are rebuilt on every frame).
    static OFFSET: Rc<Cell<f64>> = Rc::new(Cell::new(0.0));
}

/// Back to the top (the panel was opened again).
pub fn reset_scroll() {
    OFFSET.with(|o| o.set(0.0));
}

pub fn build(history: &[PastAlert], close: Rc<dyn Fn()>) -> El {
    if history.is_empty() {
        return ui::with_text(ui::text(l::t("Nothing yet. Alerts show up here, also the ones that were held back."), 13.0, NORMAL, colors::grey()), |t| t.wrap = true)
            .margin(0.0, 18.0, 0.0, 0.0)
            .valign(Align::Start);
    }
    let list = El::vstack(history.iter().map(|p| row(p, close.clone())).collect()).margin(0.0, 8.0, 0.0, 0.0);
    // The island never takes focus, but the mouse wheel still scrolls the list.
    El::new(Kind::Scroll { child: Box::new(list), offset: OFFSET.with(|o| o.clone()) })
}

fn row(past: &PastAlert, close: Rc<dyn Fn()>) -> El {
    let time = ui::digits(crate::activities::short_time(past.time), 12.0, MEDIUM, colors::grey());
    let mut runs = vec![Run { text: past.title.clone(), size: 13.5, weight: SEMIBOLD, color: colors::primary() }];
    if !past.detail.is_empty() {
        runs.push(Run { text: format!("  {}", past.detail), size: 13.0, weight: NORMAL, color: colors::grey() });
    }
    let mut cells = vec![(0, 0, time), (0, 1, ui::rich(runs))];
    if past.missed {
        let moon = ui::glyph(icons::MOON, 12.0, colors::indigo()).margin(8.0, 0.0, 0.0, 0.0).tooltip(l::t("Held back (Do Not Disturb or quiet times)"));
        cells.push((0, 2, moon));
    }
    let mut grid = El::grid(vec![Len::Px(46.0), Len::Star(1.0), Len::Auto], vec![], cells).h(34.0).bg(crate::ui::colors::Color::TRANSPARENT);
    if let Some(click) = past.on_click.clone() {
        grid = grid.cursor(Cursor::Hand).on_click(move |_, _| {
            close();
            click();
        });
    }
    grid
}
