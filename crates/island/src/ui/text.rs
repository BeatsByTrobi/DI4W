//! Text with DirectWrite, set like the C# island's TextBlocks: Apple's San Francisco when it's installed, Windows'
//! own Segoe UI Variable otherwise; icons from Segoe Fluent Icons (Segoe MDL2 Assets on Windows 10); weather
//! symbols from Segoe UI Symbol. Text that doesn't fit is trimmed with an ellipsis.

use super::el::{Font, Text, TextAlign};
use super::geom::Size;
use std::cell::OnceCell;
use windows::Win32::Graphics::DirectWrite::*;
use windows::core::{BOOL, HSTRING};

thread_local! {
    static FACTORY: OnceCell<Option<IDWriteFactory>> = const { OnceCell::new() };
    static FAMILIES: OnceCell<[HSTRING; 4]> = const { OnceCell::new() };
}

/// The shared DirectWrite factory (`None` if DirectWrite is unavailable, which it never is on Windows 10+).
pub fn factory() -> Option<IDWriteFactory> {
    FACTORY.with(|f| {
        f.get_or_init(|| unsafe { DWriteCreateFactory::<IDWriteFactory>(DWRITE_FACTORY_TYPE_SHARED).ok() }).clone()
    })
}

/// The installed family for each [`Font`], picked once: the first of the C# app's font lists that exists.
fn family(font: Font) -> HSTRING {
    FAMILIES.with(|f| {
        f.get_or_init(|| {
            let pick = |names: &[&str]| -> HSTRING {
                let installed = |name: &str| -> bool {
                    let Some(factory) = factory() else { return false };
                    unsafe {
                        let mut collection = None;
                        if factory.GetSystemFontCollection(&mut collection, false).is_err() {
                            return false;
                        }
                        let Some(collection) = collection else { return false };
                        let (mut index, mut exists) = (0u32, BOOL(0));
                        collection.FindFamilyName(&HSTRING::from(name), &mut index, &mut exists).is_ok() && exists.as_bool()
                    }
                };
                let name = names.iter().find(|n| installed(n)).unwrap_or(&names[names.len() - 1]);
                HSTRING::from(*name)
            };
            [
                pick(&["SF Pro Text", "SF Pro Display", "Segoe UI Variable Text", "Segoe UI"]),
                pick(&["Segoe Fluent Icons", "Segoe MDL2 Assets"]),
                pick(&["Segoe UI Symbol", "Segoe UI"]),
                pick(&["Segoe UI Variable Text", "Segoe UI"]),
            ]
        })[match font {
            Font::Text => 0,
            Font::Icons => 1,
            Font::Symbols => 2,
            Font::Menu => 3,
        }]
        .clone()
    })
}

/// A laid-out text, ready to draw.
#[derive(Clone)]
pub struct TextLayout {
    pub layout: Option<IDWriteTextLayout>,
    pub max_width: f64,
    pub max_height: f64,
    pub width: f64,
    pub height: f64,
    /// Where the first line's text starts horizontally (non-zero for centered/right-aligned text).
    pub left: f64,
}

impl TextLayout {
    pub fn size(&self) -> Size {
        Size::new(self.width, self.height)
    }
}

const BIG: f32 = 100_000.0;

/// Lays out the text in a box of at most `max_width` × `max_height` (either may be infinite).
pub fn layout(text: &Text, max_width: f64, max_height: f64) -> TextLayout {
    let empty = TextLayout { layout: None, max_width, max_height, width: 0.0, height: 0.0, left: 0.0 };
    let Some(factory) = factory() else { return empty };
    let Some(first) = text.runs.first() else { return empty };
    let content: Vec<u16> = text.content().encode_utf16().collect();
    let fw = if max_width.is_finite() { max_width.max(0.0) as f32 } else { BIG };
    let fh = if max_height.is_finite() { max_height.max(0.0) as f32 } else { BIG };
    unsafe {
        let Ok(format) = factory.CreateTextFormat(
            &family(text.font),
            None,
            DWRITE_FONT_WEIGHT(first.weight as i32),
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            first.size as f32,
            &HSTRING::from(""),
        ) else {
            return empty;
        };
        let _ = format.SetWordWrapping(if text.wrap { DWRITE_WORD_WRAPPING_WRAP } else { DWRITE_WORD_WRAPPING_NO_WRAP });
        let _ = format.SetTextAlignment(match text.align {
            TextAlign::Left => DWRITE_TEXT_ALIGNMENT_LEADING,
            TextAlign::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
            TextAlign::Right => DWRITE_TEXT_ALIGNMENT_TRAILING,
        });
        if text.trim {
            let trimming = DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 };
            if let Ok(sign) = factory.CreateEllipsisTrimmingSign(&format) {
                let _ = format.SetTrimming(&trimming, &sign);
            }
        }
        let Ok(layout) = factory.CreateTextLayout(&content, &format, fw, fh) else { return empty };

        // Runs after the first: their own size, weight (colors are applied when drawing).
        let mut start = 0u32;
        for run in &text.runs {
            let len = run.text.encode_utf16().count() as u32;
            let range = DWRITE_TEXT_RANGE { startPosition: start, length: len };
            let _ = layout.SetFontSize(run.size as f32, range);
            let _ = layout.SetFontWeight(DWRITE_FONT_WEIGHT(run.weight as i32), range);
            start += len;
        }
        if text.tabular
            && let Ok(typography) = factory.CreateTypography() {
                let _ = typography.AddFontFeature(DWRITE_FONT_FEATURE { nameTag: DWRITE_FONT_FEATURE_TAG_TABULAR_FIGURES, parameter: 1 });
                let _ = layout.SetTypography(&typography, DWRITE_TEXT_RANGE { startPosition: 0, length: content.len() as u32 });
            }

        let mut metrics = DWRITE_TEXT_METRICS::default();
        let _ = layout.GetMetrics(&mut metrics);
        let height = if max_height.is_finite() { (metrics.height as f64).min(max_height.max(0.0)) } else { metrics.height as f64 };
        let width = (metrics.width as f64).min(fw as f64);
        TextLayout { layout: Some(layout), max_width, max_height, width, height, left: metrics.left as f64 }
    }
}

/// Characters drawn with the icon font's private-use area: there's nothing to read there.
pub fn is_glyph(text: &str) -> bool {
    text.trim().chars().all(|c| ('\u{E000}'..='\u{F8FF}').contains(&c))
}

