//! Everything the island draws with: colors, text, icons, the element tree and its layout, the renderer, and the
//! shared building blocks every view uses (the C# app's `Ui` class), so all views share the same iOS-like
//! typography, colors and controls.

pub mod colors;
pub mod el;
pub mod geom;
pub mod icons;
pub mod image;

pub mod render;
pub mod shape;
pub mod text;


use colors::Color;
use el::{Action, Align, ButtonSpec, Cursor, El, Font, Kind, Len, Run, Text, TextAlign};
use std::rc::Rc;
use std::time::Duration;

// Apple's expanded presentation, in points: 371 wide, 84 tall at the least. Content sits at an even 16 from the
// edges, which with a 52 icon makes the icon concentric with the island's rounded end.
pub const ALERT_WIDTH: f64 = 371.0;
pub const ALERT_HEIGHT: f64 = 84.0;
pub const ICON_SIZE: f64 = 52.0;
const MARGIN: f64 = 16.0;
const ACTION_HEIGHT: f64 = 44.0;

/// Height of an alert with a row of buttons under it ([`action_alert_view`]).
pub const ACTION_ALERT_HEIGHT: f64 = MARGIN + ICON_SIZE + 12.0 + ACTION_HEIGHT + MARGIN;

// System indicators (Silent mode, Focus, charging on the iPhone) keep the island one line tall and just widen it.
pub const INDICATOR_WIDTH: f64 = 290.0;
pub const INDICATOR_HEIGHT: f64 = 37.0;

pub const LIGHT: u16 = 300;
pub const NORMAL: u16 = 400;
pub const MEDIUM: u16 = 500;
pub const SEMIBOLD: u16 = 600;

/// A capsule corner radius (half the shorter side), resolved when the element has its size.
pub const CAPSULE: f64 = -1.0;

fn text_el(runs: Vec<Run>, font: Font, tabular: bool, trim: bool) -> El {
    El::new(Kind::Text(Text { runs, font, tabular, wrap: false, align: TextAlign::Left, trim, layout: Default::default() }))
}

/// A TextBlock in the island's font, trimmed with an ellipsis and vertically centered (C#'s `Ui.Text`).
pub fn text(content: impl Into<String>, size: f64, weight: u16, color: Color) -> El {
    text_el(vec![Run { text: content.into(), size, weight, color }], Font::Text, false, true).valign(Align::Center)
}

/// [`text`] with tabular (fixed-width) digits, for times and percentages that change.
pub fn digits(content: impl Into<String>, size: f64, weight: u16, color: Color) -> El {
    text_el(vec![Run { text: content.into(), size, weight, color }], Font::Text, true, true).valign(Align::Center)
}

/// Several runs in one TextBlock (a bold title and a grey detail on one line).
pub fn rich(runs: Vec<Run>) -> El {
    text_el(runs, Font::Text, false, true).valign(Align::Center)
}

/// An icon from Segoe Fluent Icons / Segoe MDL2 Assets, centered (C#'s `Ui.Glyph`).
pub fn glyph(glyph: &str, size: f64, color: Color) -> El {
    text_el(vec![Run { text: glyph.into(), size, weight: NORMAL, color }], Font::Icons, false, false).align(Align::Center, Align::Center)
}

/// A character from Segoe UI Symbol (the weather symbols).
pub fn symbol(symbol: &str, size: f64, color: Color) -> El {
    text_el(vec![Run { text: symbol.into(), size, weight: NORMAL, color }], Font::Symbols, false, false).valign(Align::Center)
}

/// Changes a text element's options (wrapping, alignment).
pub fn with_text(mut el: El, f: impl FnOnce(&mut Text)) -> El {
    if let Kind::Text(t) = &mut el.kind {
        f(t);
    }
    el
}

/// A button with no chrome: the content is the whole button, with hover/press feedback. `w`/`h` `None` size it to
/// its content; the corners are half the shorter side (square buttons are circles, wide ones capsules).
pub fn button(key: u64, content: El, on_click: impl Fn() + 'static, background: Option<Color>, w: Option<f64>, h: Option<f64>, tooltip: Option<String>) -> El {
    // Screen readers: icon-only buttons need a name; use the tooltip, or the label text.
    let name = tooltip.clone().or_else(|| match &content.kind {
        Kind::Text(t) if !text::is_glyph(&t.content()) => Some(t.content()),
        _ => None,
    });
    let mut b = El::boxed(vec![content.align(Align::Center, Align::Center)]);
    b.background = Some(background.unwrap_or(Color::TRANSPARENT).into());
    b.radius = [CAPSULE; 4];
    b.width = w;
    b.height = h;
    b.tooltip = tooltip;
    b.name = name;
    b.cursor = Cursor::Hand;
    b.button = Some(ButtonSpec { key, action: Rc::new(on_click) as Action });
    b
}

/// Round button as used in iOS live activities (timer pause / cancel).
pub fn circle_button(key: u64, icon: El, background: Color, size: f64, on_click: impl Fn() + 'static, tooltip: Option<String>) -> El {
    button(key, icon, on_click, Some(background), Some(size), Some(size), tooltip)
}

/// A rounded picture (album art, app icons); grey while there's none.
pub fn image(image: Option<u64>, size: f64, radius: f64) -> El {
    El::empty()
        .size(size, size)
        .radius(radius)
        .bg(match image {
            Some(id) => el::Brush::Image(id),
            None => el::Brush::Solid(colors::button_bg()),
        })
        .valign(Align::Center)
}

/// The standard expanded alert, laid out like Apple's: a 52 icon at an even 16 from the edges, a heavy title, a
/// secondary-label second line, and an optional accessory (round buttons) on the trailing side.
pub fn alert_view(icon: El, title: &str, subtitle: &str, title_color: Option<Color>, accessory: Option<El>) -> El {
    let icon_host = El::boxed(vec![icon]).size(ICON_SIZE, ICON_SIZE).valign(Align::Center);
    let mut lines = vec![text(title, 16.0, SEMIBOLD, title_color.unwrap_or_else(colors::primary))];
    if !subtitle.trim().is_empty() {
        let sub = with_text(text(subtitle, 14.5, NORMAL, colors::grey()), |t| t.wrap = true).max_h(38.0).margin(0.0, 1.0, 0.0, 0.0);
        lines.push(sub);
    }
    let stack = El::vstack(lines).margin(12.0, 0.0, if accessory.is_none() { 4.0 } else { 10.0 }, 0.0).valign(Align::Center);
    let mut cells = vec![(0, 0, icon_host), (0, 1, stack)];
    if let Some(a) = accessory {
        cells.push((0, 2, El::boxed(vec![a]).valign(Align::Center)));
    }
    El::grid(vec![Len::Auto, Len::Star(1.0), Len::Auto], vec![], cells)
        .margin_all(MARGIN)
        .h(ICON_SIZE)
        .valign(Align::Start)
}

/// A one-line indicator: something on the leading side, something on the trailing side, snug against the rounded
/// ends like a compact activity.
pub fn indicator(leading: El, trailing: El) -> El {
    El::boxed(vec![
        leading.align(Align::Start, Align::Center).margin(13.0, 0.0, 0.0, 0.0),
        trailing.align(Align::End, Align::Center).margin(0.0, 0.0, 15.0, 0.0),
    ])
    .h(INDICATOR_HEIGHT)
}

/// An alert with a row of wide capsule buttons under it, like the iPhone's expanded prompts. Use with
/// [`ACTION_ALERT_HEIGHT`].
pub fn action_alert_view(icon: El, title: &str, subtitle: &str, actions: Vec<El>) -> El {
    let mut header = alert_view(icon, title, subtitle, None, None);
    header.margin = geom::Thickness::new(MARGIN, MARGIN, MARGIN, 0.0);
    let buttons = actions.into_iter().map(|a| a.margin(4.0, 0.0, 4.0, 0.0).h(ACTION_HEIGHT)).collect();
    let row = El::uniform(buttons).margin(MARGIN - 4.0, 12.0, MARGIN - 4.0, MARGIN).h(ACTION_HEIGHT);
    El::vstack(vec![header, row])
}

/// A wide capsule button for [`action_alert_view`]: grey, or filled with a color for the main action.
pub fn action_button(key: u64, label: &str, on_click: impl Fn() + 'static, fill: Option<Color>) -> El {
    let t = text(label, 16.0, SEMIBOLD, if fill.is_none() { colors::primary() } else { Color::WHITE });
    button(key, t, on_click, Some(fill.unwrap_or_else(colors::button_bg)), None, None, None).named(label)
}

/// A filled circle with a glyph in the middle (fallback app icon). White glyph on a color; on the neutral grey the
/// glyph follows the theme.
pub fn glyph_circle(g: &str, background: Color, size: f64, glyph_size: f64) -> El {
    let neutral = background == colors::button_bg() || background == colors::track();
    El::boxed(vec![glyph(g, glyph_size, if neutral { colors::primary() } else { colors::on_color() })])
        .size(size, size)
        .radius(size / 2.0)
        .bg(background)
}

/// "4:05" or "1:02:03" (negative times show as 0:00).
pub fn format_time(t: Duration) -> String {
    let secs = t.as_secs();
    if secs >= 3600 {
        format!("{}:{:02}:{:02}", secs / 3600, (secs / 60) % 60, secs % 60)
    } else {
        format!("{}:{:02}", secs / 60, secs % 60)
    }
}

/// [`format_time`] for a signed number of seconds.
pub fn format_seconds(seconds: f64) -> String {
    format_time(Duration::from_secs_f64(seconds.max(0.0)))
}

/// The readable text inside a view, for screen readers and the alert history: texts and tooltips in reading order,
/// without icon-font glyphs or repeats.
pub fn texts(root: &El) -> Vec<String> {
    fn walk(node: &El, out: &mut Vec<String>) {
        match &node.kind {
            Kind::Text(t) if !t.content().trim().is_empty() && !text::is_glyph(&t.content()) => out.push(t.content().trim().to_string()),
            _ => {
                if let Some(tip) = node.tooltip.as_ref().filter(|t| !t.trim().is_empty()) {
                    out.push(tip.clone());
                }
            }
        }
        for c in node.children() {
            walk(c, out);
        }
    }
    let mut words = Vec::new();
    walk(root, &mut words);
    let mut seen = std::collections::HashSet::new();
    words.retain(|w| seen.insert(w.clone()));
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_times_like_ios() {
        assert_eq!(format_time(Duration::from_secs(0)), "0:00");
        assert_eq!(format_time(Duration::from_secs(305)), "5:05");
        assert_eq!(format_time(Duration::from_secs(3723)), "1:02:03");
        assert_eq!(format_seconds(-4.0), "0:00");
    }

    #[test]
    fn reads_texts_and_tooltips_but_not_icons() {
        let view = alert_view(glyph_circle(icons::CHAT, colors::blue(), ICON_SIZE, 22.0), "Anna", "Hello there", None,
            Some(glyph("\u{E708}", 12.0, colors::indigo()).tooltip("Held back")));
        assert_eq!(texts(&view), vec!["Anna", "Hello there", "Held back"]);
    }

    #[test]
    fn action_alerts_are_as_tall_as_apples() {
        assert_eq!(ACTION_ALERT_HEIGHT, 16.0 + 52.0 + 12.0 + 44.0 + 16.0);
    }
}
