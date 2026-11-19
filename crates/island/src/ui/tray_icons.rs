//! The tray icon in two states: normal, and paused (faded, with an orange pause badge), so a hidden island is
//! visible at a glance in the taskbar.

use windows::Win32::Foundation::HINSTANCE;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::PCWSTR;

/// The island's icon (resource 1) at the taskbar's small-icon size for this display scale (16 px at 100%, 20 at
/// 125%, 24 at 150%…).
pub fn normal() -> HICON {
    let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16);
    unsafe {
        let instance: HINSTANCE = GetModuleHandleW(None).unwrap_or_default().into();
        LoadImageW(Some(instance), PCWSTR(std::ptr::without_provenance(1)), IMAGE_ICON, size, size, LR_DEFAULTCOLOR)
            .map(|h| HICON(h.0))
            .unwrap_or_else(|_| LoadIconW(None, IDI_APPLICATION).unwrap_or_default())
    }
}

/// Coverage (0..1) of a pixel by a circle, sampled 4×4.
fn circle_coverage(px: f64, py: f64, cx: f64, cy: f64, r: f64) -> f64 {
    let mut n = 0;
    for i in 0..4 {
        for j in 0..4 {
            let x = px + (i as f64 + 0.5) / 4.0 - cx;
            let y = py + (j as f64 + 0.5) / 4.0 - cy;
            if x * x + y * y <= r * r {
                n += 1;
            }
        }
    }
    n as f64 / 16.0
}

fn rect_coverage(px: f64, py: f64, x: f64, y: f64, w: f64, h: f64) -> f64 {
    let ox = ((px + 1.0).min(x + w) - px.max(x)).max(0.0);
    let oy = ((py + 1.0).min(y + h) - py.max(y)).max(0.0);
    ox * oy
}

/// The faded icon with an orange pause badge in the bottom-right corner.
pub fn paused() -> HICON {
    let base = normal();
    let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16);
    unsafe {
        // The icon's pixels, premultiplied BGRA, top-down.
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(Some(screen));
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size,
                biHeight: -size,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let Ok(dib) = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) else {
            let _ = DeleteDC(dc);
            ReleaseDC(None, screen);
            return base;
        };
        let old = SelectObject(dc, dib.into());
        let _ = DrawIconEx(dc, 0, 0, base, size, size, 0, None, DI_NORMAL);
        let pixels = std::slice::from_raw_parts_mut(bits as *mut u8, (size * size * 4) as usize);

        let s = size as f64;
        let badge = s * 0.62;
        let (bx, by) = (s - badge, s - badge);
        let (cx, cy, r) = (bx + (badge - 0.5) / 2.0, by + (badge - 0.5) / 2.0, (badge - 0.5) / 2.0);
        let bar_w = badge * 0.16;
        let bar_h = badge * 0.46;
        let bar_top = by + (badge - bar_h) / 2.0;
        let center = bx + badge / 2.0;
        for y in 0..size {
            for x in 0..size {
                let i = ((y * size + x) * 4) as usize;
                // Faded to 45%.
                let mut px = [pixels[i] as f64, pixels[i + 1] as f64, pixels[i + 2] as f64, pixels[i + 3] as f64];
                for c in px.iter_mut() {
                    *c *= 0.45;
                }
                let over = |px: &mut [f64; 4], color: [f64; 3], a: f64| {
                    // Source-over, premultiplied.
                    for c in 0..3 {
                        px[c] = color[c] * 255.0 * a + px[c] * (1.0 - a);
                    }
                    px[3] = 255.0 * a + px[3] * (1.0 - a);
                };
                let (fx, fy) = (x as f64, y as f64);
                let orange = circle_coverage(fx, fy, cx, cy, r);
                if orange > 0.0 {
                    over(&mut px, [0x0A as f64 / 255.0, 0x9F as f64 / 255.0, 1.0], orange); // #FF9F0A as BGR
                }
                let white = rect_coverage(fx, fy, center - bar_w * 1.5, bar_top, bar_w, bar_h) + rect_coverage(fx, fy, center + bar_w * 0.5, bar_top, bar_w, bar_h);
                if white > 0.0 {
                    over(&mut px, [1.0, 1.0, 1.0], white.min(1.0));
                }
                // Icons store straight (not premultiplied) alpha.
                let a = px[3];
                for c in 0..3 {
                    let straight = if a > 0.0 { px[c] * 255.0 / a } else { 0.0 };
                    pixels[i + c] = straight.round().clamp(0.0, 255.0) as u8;
                }
                pixels[i + 3] = a.round().clamp(0.0, 255.0) as u8;
            }
        }
        SelectObject(dc, old);
        let mask = CreateBitmap(size, size, 1, 1, None);
        let icon_info = ICONINFO { fIcon: true.into(), xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: dib };
        let icon = CreateIconIndirect(&icon_info).unwrap_or(base);
        let _ = DeleteObject(mask.into());
        let _ = DeleteObject(dib.into());
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);
        icon
    }
}
