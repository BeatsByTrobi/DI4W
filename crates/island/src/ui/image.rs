//! Pictures in the island (album art, app logos, screenshot thumbnails, copied images): decoded with WIC into
//! premultiplied BGRA pixels on any thread, then registered on the UI thread under an id that views reference with
//! [`super::el::Brush::Image`]. The renderer uploads each one to the GPU once.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::UI::Shell::SHCreateMemStream;

/// Premultiplied BGRA pixels, rows `w * 4` bytes apart.
#[derive(Clone, Debug, PartialEq)]
pub struct Bitmap {
    pub w: u32,
    pub h: u32,
    pub pixels: Vec<u8>,
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

thread_local! {
    static IMAGES: RefCell<HashMap<u64, Rc<Bitmap>>> = RefCell::new(HashMap::new());
}

/// A registered picture; dropping the handle forgets it (and the renderer frees its GPU copy).
#[derive(Debug)]
pub struct Image {
    pub id: u64,
    pub w: u32,
    pub h: u32,
}

impl Image {
    /// Registers the pixels (UI thread).
    pub fn new(bitmap: Bitmap) -> Rc<Image> {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let (w, h) = (bitmap.w, bitmap.h);
        IMAGES.with(|m| m.borrow_mut().insert(id, Rc::new(bitmap)));
        Rc::new(Image { id, w, h })
    }
}

impl Drop for Image {
    fn drop(&mut self) {
        let _ = IMAGES.try_with(|m| m.borrow_mut().remove(&self.id));
    }
}

/// The pixels of a registered picture, or `None` once it's gone.
pub fn pixels(id: u64) -> Option<Rc<Bitmap>> {
    IMAGES.with(|m| m.borrow().get(&id).cloned())
}

/// How to fit a decoded picture.
#[derive(Clone, Copy, Debug, Default)]
pub struct Decode {
    /// Decode at this width (keeping the aspect ratio), like WPF's `DecodePixelWidth`.
    pub width: Option<u32>,
    /// Decode at this height (keeping the aspect ratio), like `DecodePixelHeight`.
    pub height: Option<u32>,
    /// Crop to the centered square (album art with a letterboxed border).
    pub square: bool,
}

fn factory() -> windows::core::Result<IWICImagingFactory> {
    unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }
}

/// Decodes a PNG/JPEG/BMP/GIF file's bytes. COM must be initialized on the calling thread.
pub fn decode(bytes: &[u8], how: Decode) -> windows::core::Result<Bitmap> {
    unsafe {
        let wic = factory()?;
        let stream = SHCreateMemStream(Some(bytes)).ok_or_else(|| windows::core::Error::from(windows::Win32::Foundation::E_OUTOFMEMORY))?;
        let decoder = wic.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnLoad)?;
        from_source(&wic, &decoder.GetFrame(0)?.cast()?, how)
    }
}

/// Decodes a picture file (screenshots).
pub fn decode_file(path: &std::path::Path, how: Decode) -> windows::core::Result<Bitmap> {
    let bytes = std::fs::read(path).map_err(|e| windows::core::Error::new(windows::Win32::Foundation::E_FAIL, e.to_string()))?;
    decode(&bytes, how)
}

use windows::core::Interface;

unsafe fn from_source(wic: &IWICImagingFactory, source: &IWICBitmapSource, how: Decode) -> windows::core::Result<Bitmap> {
    unsafe {
        let (mut w, mut h) = (0u32, 0u32);
        source.GetSize(&mut w, &mut h)?;
        let mut current: IWICBitmapSource = source.clone();
        let target = match (how.width, how.height) {
            (Some(tw), _) if w > 0 => Some((tw, ((h as f64) * tw as f64 / w as f64).round().max(1.0) as u32)),
            (None, Some(th)) if h > 0 => Some((((w as f64) * th as f64 / h as f64).round().max(1.0) as u32, th)),
            _ => None,
        };
        if let Some((tw, th)) = target
            && (tw, th) != (w, h) {
                let scaler = wic.CreateBitmapScaler()?;
                scaler.Initialize(&current, tw, th, WICBitmapInterpolationModeFant)?;
                current = scaler.cast()?;
                (w, h) = (tw, th);
            }
        if how.square && w != h {
            let side = w.min(h);
            let clipper = wic.CreateBitmapClipper()?;
            let rect = WICRect { X: ((w - side) / 2) as i32, Y: ((h - side) / 2) as i32, Width: side as i32, Height: side as i32 };
            clipper.Initialize(&current, &rect)?;
            current = clipper.cast()?;
            (w, h) = (side, side);
        }
        let converter = wic.CreateFormatConverter()?;
        converter.Initialize(&current, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)?;
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        converter.CopyPixels(std::ptr::null(), w * 4, &mut pixels)?;
        Ok(Bitmap { w, h, pixels })
    }
}

/// A smaller copy (box filter), for thumbnails of big clipboard images and color sampling.
pub fn shrink(b: &Bitmap, max_side: u32) -> Bitmap {
    let scale = (max_side as f64 / b.w.max(b.h).max(1) as f64).min(1.0);
    let (w, h) = (((b.w as f64 * scale).round() as u32).max(1), ((b.h as f64 * scale).round() as u32).max(1));
    resample(b, w, h)
}

/// Resamples to exactly `w` × `h` by averaging the source pixels each target pixel covers.
pub fn resample(b: &Bitmap, w: u32, h: u32) -> Bitmap {
    if (w, h) == (b.w, b.h) || b.w == 0 || b.h == 0 {
        return b.clone();
    }
    let mut out = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        let y0 = (y as u64 * b.h as u64 / h as u64) as u32;
        let y1 = (((y + 1) as u64 * b.h as u64).div_ceil(h as u64) as u32).clamp(y0 + 1, b.h);
        for x in 0..w {
            let x0 = (x as u64 * b.w as u64 / w as u64) as u32;
            let x1 = (((x + 1) as u64 * b.w as u64).div_ceil(w as u64) as u32).clamp(x0 + 1, b.w);
            let mut sum = [0u64; 4];
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let i = ((sy * b.w + sx) * 4) as usize;
                    for c in 0..4 {
                        sum[c] += b.pixels[i + c] as u64;
                    }
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u64;
            let o = ((y * w + x) * 4) as usize;
            for c in 0..4 {
                out[o + c] = (sum[c] / n) as u8;
            }
        }
    }
    Bitmap { w, h, pixels: out }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampling_averages() {
        let b = Bitmap { w: 2, h: 2, pixels: vec![0, 0, 0, 255, 255, 255, 255, 255, 255, 255, 255, 255, 0, 0, 0, 255] };
        let one = resample(&b, 1, 1);
        assert_eq!(one.pixels, vec![127, 127, 127, 255]);
        assert_eq!(shrink(&b, 4).w, 2);
    }
}
