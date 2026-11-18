//! Captures what the speakers are playing (WASAPI loopback) and turns it into a few smoothed frequency bands for
//! the waveform bars. Only runs while music is playing.

use std::f64::consts::PI;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, CoTaskMemFree};

pub const BAND_COUNT: usize = 6;
const FFT_LOG2: u32 = 10;
const FFT_SIZE: usize = 1 << FFT_LOG2;

struct Levels {
    ring: Vec<f32>,
    pos: usize,
    bands: [f64; BAND_COUNT],
    peak: [f64; BAND_COUNT],
    last_data: Option<Instant>,
    sample_rate: f64,
}

static LEVELS: Mutex<Option<Levels>> = Mutex::new(None);
static FAILED: AtomicBool = AtomicBool::new(false);
static RUNNING: Mutex<Option<Arc<AtomicBool>>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut Levels) -> R) -> R {
    let mut guard = LEVELS.lock().unwrap_or_else(|e| e.into_inner());
    let levels = guard.get_or_insert_with(|| Levels {
        ring: vec![0.0; FFT_SIZE],
        pos: 0,
        bands: [0.0; BAND_COUNT],
        peak: [0.05; BAND_COUNT],
        last_data: None,
        sample_rate: 48_000.0,
    });
    f(levels)
}

/// Loopback capture couldn't start; the bars fall back to a gentle simulated motion.
pub fn failed() -> bool {
    FAILED.load(Ordering::Relaxed)
}

/// True when audio frames arrived recently (loopback delivers nothing during silence).
pub fn has_signal() -> bool {
    with(|l| l.last_data.is_some_and(|t| t.elapsed() < Duration::from_millis(150)))
}

/// The current band levels (0..1).
pub fn read() -> [f64; BAND_COUNT] {
    with(|l| l.bands)
}

pub fn start() {
    if failed() {
        return;
    }
    let mut running = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
    if running.is_some() {
        return;
    }
    let flag = Arc::new(AtomicBool::new(true));
    *running = Some(flag.clone());
    crate::rt::background(move || {
        if let Err(e) = capture(&flag) {
            di4w_core::log::error_with("Loopback capture unavailable; falling back to simulated bars", e);
            FAILED.store(true, Ordering::Relaxed);
        }
    });
}

pub fn stop() {
    if let Some(flag) = RUNNING.lock().unwrap_or_else(|e| e.into_inner()).take() {
        flag.store(false, Ordering::SeqCst);
    }
}

fn capture(running: &AtomicBool) -> windows::core::Result<()> {
    unsafe {
        let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let device = enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia)?;
        let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
        let format = client.GetMixFormat()?;
        let f = *format;
        // Shared-mode loopback is 32-bit float in practice (often wrapped as WAVE_FORMAT_EXTENSIBLE).
        let is_float = f.wFormatTag == 3 || (f.wFormatTag == 0xFFFE && f.wBitsPerSample == 32);
        let channels = f.nChannels.max(1) as usize;
        let bytes_per_sample = (f.wBitsPerSample / 8) as usize;
        with(|l| l.sample_rate = f.nSamplesPerSec as f64);
        let init = client.Initialize(AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK, 1_000_000, 0, format, None);
        CoTaskMemFree(Some(format as *const _));
        init?;
        let capture: IAudioCaptureClient = client.GetService()?;
        client.Start()?;
        while running.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(50));
            loop {
                let packet = capture.GetNextPacketSize()?;
                if packet == 0 {
                    break;
                }
                let mut data: *mut u8 = std::ptr::null_mut();
                let mut frames = 0u32;
                let mut flags = 0u32;
                capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
                let frame_bytes = bytes_per_sample * channels;
                let silent = flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0;
                if frames > 0 {
                    let bytes = std::slice::from_raw_parts(data, frames as usize * frame_bytes);
                    let mut samples = Vec::with_capacity(frames as usize);
                    for frame in bytes.chunks_exact(frame_bytes) {
                        let mut sum = 0f32;
                        if !silent {
                            for c in 0..channels {
                                let o = c * bytes_per_sample;
                                sum += if is_float {
                                    f32::from_le_bytes([frame[o], frame[o + 1], frame[o + 2], frame[o + 3]])
                                } else if bytes_per_sample == 2 {
                                    i16::from_le_bytes([frame[o], frame[o + 1]]) as f32 / 32768.0
                                } else {
                                    0.0
                                };
                            }
                        }
                        samples.push(sum / channels as f32);
                    }
                    feed(&samples);
                }
                capture.ReleaseBuffer(frames)?;
            }
        }
        let _ = client.Stop();
        Ok(())
    }
}

fn feed(samples: &[f32]) {
    with(|l| {
        l.last_data = Some(Instant::now());
        for &s in samples {
            let pos = l.pos;
            l.ring[pos] = s;
            l.pos += 1;
            if l.pos == FFT_SIZE {
                l.pos = 0;
                analyze(l);
            }
        }
    });
}

/// In-place radix-2 FFT; the forward transform is scaled by 1/n (as NAudio's is).
fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let angle = -2.0 * PI / len as f64;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (wr, wi) = ((angle * k as f64).cos(), (angle * k as f64).sin());
                let (a, b) = (start + k, start + k + len / 2);
                let tr = re[b] * wr - im[b] * wi;
                let ti = re[b] * wi + im[b] * wr;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len <<= 1;
    }
    for i in 0..n {
        re[i] /= n as f64;
        im[i] /= n as f64;
    }
}

fn analyze(l: &mut Levels) {
    let mut re: Vec<f64> = (0..FFT_SIZE).map(|i| l.ring[i] as f64 * 0.5 * (1.0 - (2.0 * PI * i as f64 / (FFT_SIZE - 1) as f64).cos())).collect();
    let mut im = vec![0.0; FFT_SIZE];
    fft(&mut re, &mut im);

    // Log-spaced bands between 60 Hz and 10 kHz.
    let (lo, hi) = (60.0f64, 10_000.0f64);
    let bin_hz = l.sample_rate / FFT_SIZE as f64;
    for b in 0..BAND_COUNT {
        let f0 = lo * (hi / lo).powf(b as f64 / BAND_COUNT as f64);
        let f1 = lo * (hi / lo).powf((b + 1) as f64 / BAND_COUNT as f64);
        let i0 = ((f0 / bin_hz) as usize).max(1);
        let i1 = ((f1 / bin_hz) as usize).max(i0 + 1);
        let mut sum = 0.0;
        for i in i0..i1.min(FFT_SIZE / 2) {
            sum += (re[i] * re[i] + im[i] * im[i]).sqrt();
        }
        let level = sum / (i1 - i0) as f64;
        // Auto-gain per band so quiet and loud tracks both move the bars nicely.
        l.peak[b] = level.max(l.peak[b] * 0.995);
        let v = (level / l.peak[b].max(1e-4)).clamp(0.0, 1.0).powf(0.8);
        l.bands[b] = if v > l.bands[b] { v } else { l.bands[b] * 0.7 + v * 0.3 };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fft_finds_a_tone() {
        let n = 64;
        let mut re: Vec<f64> = (0..n).map(|i| (2.0 * PI * 8.0 * i as f64 / n as f64).cos()).collect();
        let mut im = vec![0.0; n];
        fft(&mut re, &mut im);
        let peak = (0..n / 2).max_by(|a, b| re[*a].hypot(im[*a]).total_cmp(&re[*b].hypot(im[*b]))).unwrap();
        assert_eq!(peak, 8);
        assert!((re[8].hypot(im[8]) - 0.5).abs() < 1e-9); // scaled by 1/n
    }
}
