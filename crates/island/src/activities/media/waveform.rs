//! The little animated bars on the right of the pill, tinted with the album art color: the live audio level per
//! frequency band, or a gentle simulated motion when loopback capture isn't available.

use super::meter::{self, BAND_COUNT};
use crate::model::power;
use crate::rt::Timer;
use crate::ui::colors::Color;
use crate::ui::el::{Brush, El};
use crate::ui::geom::Rect;
use std::cell::RefCell;
use std::time::Duration;

// Bar order: put the mid bands in the middle so it looks like the iOS waveform.
const ORDER: [usize; BAND_COUNT] = [3, 1, 0, 2, 4, 5];

struct State {
    shared: [f64; BAND_COUNT],
    shown: [f64; BAND_COUNT],
    playing: bool,
    timer: Option<Timer>,
    seed: u64,
}

thread_local! {
    static STATE: RefCell<State> = const { RefCell::new(State { shared: [0.0; BAND_COUNT], shown: [0.0; BAND_COUNT], playing: false, timer: None, seed: 0x9E37_79B9_7F4A_7C15 }) };
}

/// 30 redraws a second normally; 10 while battery saver is on.
fn interval() -> Duration {
    Duration::from_millis(if power::saving() { 100 } else { 33 })
}

pub fn set_playing(playing: bool) {
    if playing { meter::start() } else { meter::stop() }
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.playing = playing;
        if s.timer.is_none() {
            s.timer = Some(Timer::every(interval(), tick));
        }
    });
}

fn random(seed: &mut u64) -> f64 {
    // xorshift64*
    *seed ^= *seed >> 12;
    *seed ^= *seed << 25;
    *seed ^= *seed >> 27;
    (seed.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
}

fn tick() {
    let stop = STATE.with(|s| {
        let mut s = s.borrow_mut();
        let s = &mut *s;
        if s.playing && meter::has_signal() {
            s.shared = meter::read();
        } else if s.playing && meter::failed() {
            // No loopback available: gentle fake motion.
            for i in 0..BAND_COUNT {
                s.shared[i] = s.shared[i] * 0.6 + random(&mut s.seed) * 0.4;
            }
        } else {
            for v in s.shared.iter_mut() {
                *v *= 0.8;
            }
        }
        let mut settled = true;
        for i in 0..BAND_COUNT {
            let target = s.shared[ORDER[i]];
            s.shown[i] += (target - s.shown[i]) * 0.45;
            if (target - s.shown[i]).abs() > 0.01 {
                settled = false;
            }
        }
        let stop = !s.playing && settled;
        if stop {
            s.timer = None;
        } else if let Some(t) = &s.timer {
            t.set_interval(interval());
        }
        stop
    });
    let _ = stop;
    crate::activities::redraw();
}

/// The bars in a `width` × `height` box, in the art's color fading lighter towards the bottom.
pub fn view(width: f64, height: f64, color: Color) -> El {
    let shown = STATE.with(|s| s.borrow().shown);
    let lighter = Color::rgb(color.r.saturating_add(60), color.g.saturating_add(60), color.b.saturating_add(60));
    El::custom(move |p, r: Rect| {
        let n = BAND_COUNT as f64;
        let gap = r.w * 0.12;
        let bar_w = (r.w - gap * (n - 1.0)) / n;
        let min_h = bar_w;
        for (i, v) in shown.iter().enumerate() {
            let h = min_h + (r.h - min_h) * v.clamp(0.0, 1.0);
            let x = r.x + i as f64 * (bar_w + gap);
            let y = r.y + (r.h - h) / 2.0;
            p.fill_rounded(Rect::new(x, y, bar_w, h), bar_w / 2.0, Brush::Vertical(color, lighter));
        }
    })
    .size(width, height)
}
