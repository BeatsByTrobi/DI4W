//! Vector icons drawn to resemble SF Symbols, the icon-font glyphs the island uses, the iOS-style battery and
//! the progress ring.

use super::colors::{self, Color};
use super::el::{Align, El, Kind, Shape};
use super::geom::Path;
use std::rc::Rc;
use std::sync::OnceLock;

pub const TIMER: &str = "\u{E916}"; // Stopwatch
pub const CHAT: &str = "\u{E8BD}"; // Message
pub const UPDATE: &str = "\u{E896}"; // Download
pub const BELL: &str = "\u{EA8F}"; // Ringer
pub const MOON: &str = "\u{E708}"; // Quiet hours
pub const RECENT: &str = "\u{E81C}"; // History
pub const FOCUS: &str = "\u{E7C1}"; // Flag
pub const WARNING: &str = "\u{E7BA}";
pub const CHECK: &str = "\u{E73E}";
pub const CANCEL: &str = "\u{E711}";
pub const SETTINGS: &str = "\u{E713}";
pub const VIEW: &str = "\u{E7B3}";
pub const HIDE: &str = "\u{ED1A}";
pub const STOP: &str = "\u{E71A}";
pub const PLAY_GLYPH: &str = "\u{E823}"; // Clock (start focus)
pub const SYNC: &str = "\u{E895}";
pub const POWER: &str = "\u{E7E8}";

macro_rules! icon {
    ($name:ident, $data:expr) => {
        pub fn $name() -> Rc<Path> {
            static CELL: OnceLock<Path> = OnceLock::new();
            Rc::new(CELL.get_or_init(|| Path::parse($data)).clone())
        }
    };
}

icon!(play, "M 1,0 L 13,7.5 L 1,15 Z");
icon!(pause, "M 0,0 H 4.5 V 15 H 0 Z M 8.5,0 H 13 V 15 H 8.5 Z");
icon!(next, "M 0,0 L 9,6.5 L 0,13 Z M 9,0 L 18,6.5 L 9,13 Z");
icon!(previous, "M 18,0 L 9,6.5 L 18,13 Z M 9,0 L 0,6.5 L 9,13 Z");
icon!(close, "M 0,0 L 12,12 M 12,0 L 0,12");
icon!(check, "M 0,7 L 5,12 L 14,1");
icon!(bolt, "M 6,0 L 0,8 H 4 L 2,14 L 9,5 H 5 Z");

/// A filled icon (C#'s `Ui.Shape`): fill and a 1.6 outline with round joins, stretched uniformly into a square.
pub fn shape(path: Rc<Path>, size: f64, color: Color) -> El {
    El::new(Kind::Shape(Shape { path, fill: Some(color), stroke: Some(color), thickness: 1.6, round_caps: false, stretch: true }))
        .size(size, size)
        .align(Align::Center, Align::Center)
}

/// Stroke-only icon (close / check) with round caps (C#'s `Icons.Stroke`).
pub fn stroke(path: Rc<Path>, size: f64, color: Color, thickness: f64) -> El {
    El::new(Kind::Shape(Shape { path, fill: None, stroke: Some(color), thickness, round_caps: true, stretch: true }))
        .size(size, size)
        .align(Align::Center, Align::Center)
}

/// Circular progress arc starting at 12 o'clock, clockwise; `progress` is the filled part 0..1.
pub fn ring(progress: f64, size: f64, thickness: f64, color: Color) -> El {
    El::new(Kind::Ring { progress, thickness, color }).size(size, size)
}

/// A progress ring on its grey track (the focus timer, Island API progress).
pub fn ring_on_track(progress: f64, size: f64, thickness: f64, color: Color) -> El {
    El::boxed(vec![
        El::new(Kind::Ellipse { stroke: colors::track(), thickness }).size(size, size),
        ring(progress, size, thickness, color),
    ])
    .size(size, size)
}

/// iOS-style battery: outline, fill for the charge level, a nub, and a bolt while charging.
pub fn battery(fraction: f64, charging: bool, fill: Color, width: f64) -> El {
    let height = width * 0.48;
    let border = 1.2;
    let padding = 1.6;
    let level = El::empty()
        .w((2.0f64).max((width - 6.4) * fraction.clamp(0.0, 1.0)))
        .halign(Align::Start)
        .radius(height * 0.18)
        .bg(fill);
    let mut body = El::boxed(vec![level]).size(width, height).radius(height * 0.3).border(colors::grey(), border);
    body.padding = super::geom::Thickness::all(border + padding);
    let nub = El::empty().size(2.0, height * 0.36).margin(1.0, 0.0, 0.0, 0.0).radii(0.0, 1.0, 1.0, 0.0).bg(colors::grey()).valign(Align::Center);
    let mut layers = vec![El::hstack(vec![body, nub])];
    if charging {
        let bh = height * 0.95;
        let bw = (bh - 0.8) * 9.0 / 14.0 + 0.8;
        let bolt = El::new(Kind::Shape(Shape { path: bolt(), fill: Some(colors::on_color()), stroke: Some(Color::BLACK), thickness: 0.8, round_caps: false, stretch: true }))
            .size(bw, bh)
            .margin(0.0, 0.0, 3.0, 0.0)
            .align(Align::Center, Align::Center);
        layers.push(bolt);
    }
    El::boxed(layers).valign(Align::Center)
}
