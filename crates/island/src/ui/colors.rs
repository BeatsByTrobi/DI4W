//! The island's colors: iOS' system colors, in a dark (black pill, the default) and a light version.
//!
//! Whether the island is light is set once at startup, before any view is built; switching the theme restarts the
//! island (as the C# app did, because its views were built once and reused).

use std::sync::atomic::{AtomicBool, Ordering};

static LIGHT: AtomicBool = AtomicBool::new(false);

/// True when the island uses its light colors.
pub fn light() -> bool {
    LIGHT.load(Ordering::Relaxed)
}

pub fn set_light(light: bool) {
    LIGHT.store(light, Ordering::Relaxed);
}

/// An sRGB color with straight (not premultiplied) alpha, like WPF's `Color`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Color {
    pub a: u8,
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const fn argb(a: u8, r: u8, g: u8, b: u8) -> Self {
        Self { a, r, g, b }
    }

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { a: 0xFF, r, g, b }
    }

    pub const WHITE: Color = Color::rgb(0xFF, 0xFF, 0xFF);
    pub const BLACK: Color = Color::rgb(0, 0, 0);
    pub const TRANSPARENT: Color = Color::argb(0, 0xFF, 0xFF, 0xFF);

    /// "#RRGGBB" or "#AARRGGBB".
    pub fn hex(text: &str) -> Option<Color> {
        let digits = text.strip_prefix('#')?;
        if !digits.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let v = u32::from_str_radix(digits, 16).ok()?;
        match digits.len() {
            6 => Some(Color::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)),
            8 => Some(Color::argb((v >> 24) as u8, (v >> 16) as u8, (v >> 8) as u8, v as u8)),
            _ => None,
        }
    }

    /// For constants: panics on a malformed literal (caught by the tests).
    pub fn of(text: &str) -> Color {
        Color::hex(text).unwrap_or_else(|| panic!("bad color {text}"))
    }


    /// The same color with its alpha multiplied by `opacity` (0..1).
    pub fn faded(self, opacity: f64) -> Color {
        Color { a: (self.a as f64 * opacity.clamp(0.0, 1.0)).round() as u8, ..self }
    }
}

fn themed(dark: &str, light_hex: &str) -> Color {
    Color::of(if light() { light_hex } else { dark })
}

/// The pill itself: black, or iOS' light grey-white.
pub fn pill_background() -> Color {
    themed("#000000", "#F5F5F7")
}

/// Main text and icons on the pill: white on the dark island, near-black on the light one.
pub fn primary() -> Color {
    themed("#FFFFFF", "#1C1C1E")
}

/// Icons on a colored circle or button (always white).
pub fn on_color() -> Color {
    Color::WHITE
}

/// iOS' secondaryLabel: the second line of an alert, captions, times.
pub fn grey() -> Color {
    themed("#99EBEBF5", "#993C3C43")
}

pub fn orange() -> Color {
    Color::of("#FF9F0A")
}

pub fn orange_dim() -> Color {
    themed("#4A3410", "#FFE2B8")
}

pub fn amber_dim() -> Color {
    themed("#4A4010", "#FFF1B0")
}

pub fn green() -> Color {
    Color::of("#30D158")
}

pub fn amber() -> Color {
    Color::of("#FFD60A")
}

pub fn track() -> Color {
    themed("#3A3A3C", "#D1D1D6")
}

pub fn button_bg() -> Color {
    themed("#2C2C2E", "#E5E5EA")
}

pub fn discord_blurple() -> Color {
    Color::of("#5865F2")
}

pub fn blue() -> Color {
    Color::of("#0A84FF")
}

pub fn red() -> Color {
    Color::of("#FF453A")
}

pub fn purple() -> Color {
    Color::of("#BF5AF2")
}

pub fn teal() -> Color {
    Color::of("#64D2FF")
}

pub fn indigo() -> Color {
    Color::of("#5E5CE6")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_hex_colors() {
        assert_eq!(Color::hex("#0A84FF"), Some(Color::rgb(0x0A, 0x84, 0xFF)));
        assert_eq!(Color::hex("#99EBEBF5"), Some(Color::argb(0x99, 0xEB, 0xEB, 0xF5)));
        assert_eq!(Color::hex("#12345"), None);
        assert_eq!(Color::hex("#GGGGGG"), None);
        assert_eq!(Color::hex("0A84FF"), None);
    }

    #[test]
    fn every_palette_color_parses() {
        for light in [false, true] {
            set_light(light);
            for f in [pill_background, primary, grey, orange_dim, amber_dim, track, button_bg] {
                f();
            }
        }
        set_light(false);
    }
}
