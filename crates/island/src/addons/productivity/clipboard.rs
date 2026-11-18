//! A brief "Copied" with a preview of the text or image. Respects the markers password managers put on secrets,
//! so copied passwords are never shown.

use crate::addons::{Addon, Base, Context};
use crate::model::Alert;
use crate::rt::WideNull;
use crate::ui::colors;
use crate::ui::image::{self, Bitmap, Image};
use crate::ui::{self, ICON_SIZE};
use di4w_core::{AddonSettings, l};
use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Ole::{CF_DIB, CF_HDROP, CF_UNICODETEXT};
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
use windows::core::PCWSTR;

const WM_CLIPBOARDUPDATE: u32 = 0x031D;

// Formats that mean "don't show or keep this" (KeePass, 1Password, Bitwarden, Windows itself set these).
const SECRET_MARKERS: [&str; 3] = ["ExcludeClipboardContentFromMonitorProcessing", "Clipboard Viewer Ignore", "CanIncludeInClipboardHistory"];

thread_local! {
    /// Set by the screenshot add-on; an image copied right after a screenshot isn't announced twice.
    pub static LAST_SCREENSHOT: Cell<Option<Instant>> = const { Cell::new(None) };
}

enum Content {
    Text(String),
    Image(Bitmap, u32, u32),
    Files(Vec<String>),
}

/// The clipboard, opened for reading (retries briefly when another app holds it), closed when dropped.
struct Open;

impl Open {
    fn new(hwnd: HWND) -> Option<Open> {
        for _ in 0..5 {
            if unsafe { OpenClipboard(Some(hwnd)) }.is_ok() {
                return Some(Open);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        None
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

fn format(name: &str) -> u32 {
    let wide = name.encode_wide_null();
    unsafe { RegisterClipboardFormatW(PCWSTR(wide.as_ptr())) }
}

fn data(format: u32) -> Option<Vec<u8>> {
    unsafe {
        let handle: HANDLE = GetClipboardData(format).ok()?;
        let global = HGLOBAL(handle.0);
        let ptr = GlobalLock(global);
        if ptr.is_null() {
            return None;
        }
        let bytes = std::slice::from_raw_parts(ptr as *const u8, GlobalSize(global)).to_vec();
        let _ = GlobalUnlock(global);
        Some(bytes)
    }
}

fn is_secret() -> bool {
    SECRET_MARKERS.iter().any(|m| {
        let f = format(m);
        if f == 0 || unsafe { IsClipboardFormatAvailable(f) }.is_err() {
            return false;
        }
        if *m != "CanIncludeInClipboardHistory" {
            return true;
        }
        // This one is a DWORD: 0 means "keep out of history" (used for passwords).
        data(f).is_some_and(|d| d.len() >= 4 && u32::from_le_bytes([d[0], d[1], d[2], d[3]]) == 0)
    })
}

/// A CF_DIB (BITMAPINFOHEADER + pixels) as a thumbnail and the full size.
fn decode_dib(bytes: &[u8]) -> Option<(Bitmap, u32, u32)> {
    let rd32 = |o: usize| -> Option<i64> { Some(i32::from_le_bytes(bytes.get(o..o + 4)?.try_into().ok()?) as i64) };
    let header: i64 = rd32(0)?;
    let width: i64 = rd32(4)?;
    let height: i64 = rd32(8)?;
    let bits = u16::from_le_bytes(bytes.get(14..16)?.try_into().ok()?);
    let compression: i64 = rd32(16)?;
    let colors_used: i64 = rd32(32)?;
    if width <= 0 || height == 0 || !(bits == 24 || bits == 32) {
        return None;
    }
    let (w, h) = (width as usize, height.unsigned_abs() as usize);
    let masks = if compression == 3 && header == 40 { 12 } else { 0 };
    let offset = header as usize + masks + colors_used.max(0) as usize * 4;
    let stride = (w * bits as usize / 8).div_ceil(4) * 4;
    let mut pixels = vec![0u8; w * h * 4];
    let has_alpha = bits == 32 && (0..h).any(|y| (0..w).any(|x| bytes.get(offset + y * stride + x * 4 + 3).is_some_and(|a| *a != 0)));
    for y in 0..h {
        let src_row = if height > 0 { h - 1 - y } else { y };
        for x in 0..w {
            let s = offset + src_row * stride + x * (bits as usize / 8);
            let px = bytes.get(s..s + bits as usize / 8)?;
            let a = if has_alpha { px[3] } else { 255 };
            let o = (y * w + x) * 4;
            for c in 0..3 {
                pixels[o + c] = (px[c] as u32 * a as u32 / 255) as u8;
            }
            pixels[o + 3] = a;
        }
    }
    let full = Bitmap { w: w as u32, h: h as u32, pixels };
    Some((image::shrink(&full, 128), w as u32, h as u32))
}

fn read(hwnd: HWND) -> Option<Content> {
    let _open = Open::new(hwnd)?;
    if is_secret() {
        return None;
    }
    unsafe {
        if IsClipboardFormatAvailable(CF_UNICODETEXT.0 as u32).is_ok() {
            let bytes = data(CF_UNICODETEXT.0 as u32)?;
            let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&u| u != 0).collect();
            return Some(Content::Text(String::from_utf16_lossy(&units)));
        }
        if IsClipboardFormatAvailable(CF_DIB.0 as u32).is_ok() {
            let bytes = data(CF_DIB.0 as u32)?;
            let (thumb, w, h) = decode_dib(&bytes)?;
            return Some(Content::Image(thumb, w, h));
        }
        if IsClipboardFormatAvailable(CF_HDROP.0 as u32).is_ok() {
            let handle = GetClipboardData(CF_HDROP.0 as u32).ok()?;
            let drop = HDROP(handle.0);
            let count = DragQueryFileW(drop, u32::MAX, None);
            let files = (0..count)
                .map(|i| {
                    let mut buf = vec![0u16; 1024];
                    let n = DragQueryFileW(drop, i, Some(&mut buf));
                    String::from_utf16_lossy(&buf[..n as usize])
                })
                .collect();
            return Some(Content::Files(files));
        }
    }
    None
}

pub struct Clipboard {
    base: Base,
    hook: Cell<Option<u64>>,
    last_shown: Cell<Option<Instant>>,
}

impl Clipboard {
    pub fn new(ctx: &Rc<Context>) -> Rc<Clipboard> {
        Rc::new(Clipboard { base: Base::new(ctx, "Clipboard", "Clipboard"), hook: Cell::new(None), last_shown: Cell::new(None) })
    }

    fn announce(self: &Rc<Self>) {
        if self.last_shown.get().is_some_and(|t| t.elapsed() < Duration::from_millis(400)) {
            return; // apps often write twice
        }
        match read(self.base.ctx.island.hwnd()) {
            Some(Content::Text(text)) => {
                let text = text.trim();
                if text.is_empty() {
                    return;
                }
                let preview = text.split_whitespace().collect::<Vec<_>>().join(" ");
                self.show_copied(None, l::t("Copied").into(), preview);
            }
            Some(Content::Image(thumb, w, h)) => {
                // Let the screenshot add-on claim it first.
                let me = Rc::downgrade(self);
                crate::rt::after(Duration::from_millis(600), move || {
                    let Some(c) = me.upgrade() else { return };
                    if LAST_SCREENSHOT.with(|t| t.get()).is_some_and(|t| t.elapsed() < Duration::from_secs(3)) {
                        return;
                    }
                    c.show_copied(Some(Image::new(thumb)), l::t("Copied image").into(), format!("{w} × {h}"));
                });
            }
            Some(Content::Files(files)) if !files.is_empty() => {
                let name = std::path::Path::new(&files[0]).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let detail = if files.len() == 1 { name } else { l::f("{0} and {1} more", &[&name, &(files.len() - 1)]) };
                self.show_copied(None, l::t("Copied").into(), detail);
            }
            _ => {}
        }
    }

    fn show_copied(&self, image: Option<Rc<Image>>, title: String, preview: String) {
        self.last_shown.set(Some(Instant::now()));
        let alert = Alert::new(move || {
            let icon = match &image {
                Some(i) => ui::image(Some(i.id), ICON_SIZE, 12.0),
                None => ui::glyph_circle("\u{E8C8}", colors::button_bg(), ICON_SIZE, 22.0),
            };
            ui::alert_view(icon, &title, &preview, None, None)
        })
        .seconds(2.0);
        self.base.show(alert.rc());
    }
}

impl Addon for Clipboard {
    fn base(&self) -> &Base {
        &self.base
    }

    fn enabled(&self, o: &AddonSettings) -> bool {
        o.clipboard
    }

    fn start(self: Rc<Self>) -> Result<(), String> {
        unsafe { AddClipboardFormatListener(self.base.ctx.island.hwnd()) }.map_err(|_| "Clipboard listener refused".to_string())?;
        let weak = Rc::downgrade(&self);
        let id = self.base.ctx.island.add_hook(move |msg, _, _| {
            if msg == WM_CLIPBOARDUPDATE {
                // Read after the copying app has finished writing all its formats.
                let w = weak.clone();
                crate::rt::defer(move || {
                    if let Some(c) = w.upgrade() {
                        c.announce();
                    }
                });
            }
        });
        self.hook.set(Some(id));
        Ok(())
    }

    fn stop(&self) {
        if let Some(id) = self.hook.take() {
            self.base.ctx.island.remove_hook(id);
        }
        unsafe {
            let _ = RemoveClipboardFormatListener(self.base.ctx.island.hwnd());
        }
    }

    fn preview(self: Rc<Self>) {
        self.show_copied(None, l::t("Copied").into(), l::t("A preview of the text you copy shows up here").into());
    }
}
