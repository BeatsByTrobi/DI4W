//! Springs and easing curves for the island's motion.
//!
//! The island's size, corner radius, press squeeze and the bubble all follow damped springs (as SwiftUI and the
//! iPhone's Dynamic Island do); content fades in with WPF-style eased animations (cubic ease-out, back ease-out).

use std::f64::consts::PI;

/// A damped spring (Hooke + viscous damping), integrated with small fixed substeps.
/// Slightly under-damped by default, which gives the iOS "blobby" overshoot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spring {
    pub value: f64,
    pub target: f64,
    pub velocity: f64,
    pub stiffness: f64,
    pub damping: f64,
}

impl Spring {
    pub fn new(initial: f64, stiffness: f64, damping: f64) -> Self {
        Self { value: initial, target: initial, velocity: 0.0, stiffness, damping }
    }

    /// A spring described the way SwiftUI describes them: `response` is roughly how long one swing takes
    /// (seconds), `damping_fraction` 1 means no overshoot, lower values bounce.
    pub fn from_response(initial: f64, response: f64, damping_fraction: f64) -> Self {
        let stiffness = (2.0 * PI / response).powi(2);
        let damping = 4.0 * PI * damping_fraction / response;
        Self::new(initial, stiffness, damping)
    }

    pub fn is_settled(&self) -> bool {
        (self.value - self.target).abs() < 0.02 && self.velocity.abs() < 0.02
    }


    /// Advances the spring by `dt` seconds in 1/480 s steps; once close enough it snaps to the target.
    pub fn step(&mut self, mut dt: f64) {
        const H: f64 = 1.0 / 480.0;
        while dt > 0.0 {
            let s = H.min(dt);
            let force = -self.stiffness * (self.value - self.target) - self.damping * self.velocity;
            self.velocity += force * s;
            self.value += self.velocity * s;
            dt -= s;
        }
        if self.is_settled() {
            self.value = self.target;
            self.velocity = 0.0;
        }
    }
}

/// WPF's `CubicEase` with `EasingMode.EaseOut`.
pub fn cubic_out(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// WPF's `BackEase` with `EasingMode.EaseOut`: overshoots by an amount set by `amplitude`, then settles at 1.
pub fn back_out(t: f64, amplitude: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    // WPF's BackEase.EaseInCore: t³ − t·a·sin(πt); EaseOut mirrors it.
    let ease_in = |x: f64| x.powi(3) - x * amplitude.max(0.0) * (x * PI).sin();
    1.0 - ease_in(1.0 - t)
}


/// A value going from `from` to `to` over `duration` seconds after `delay`, shaped by an easing curve.
#[derive(Clone, Copy, Debug)]
pub struct Tween {
    /// The value until the tween begins (WPF holds the previous value during an animation's BeginTime).
    pub before: f64,
    pub from: f64,
    pub to: f64,
    pub start: f64,
    pub duration: f64,
    pub ease: Ease,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ease {
    Linear,
    CubicOut,
    BackOut(f64),
}

impl Tween {
    /// `now` and `start` are seconds on the same clock; the tween begins at `start` (which may be in the future).
    pub fn new(from: f64, to: f64, start: f64, duration: f64, ease: Ease) -> Self {
        Self { before: from, from, to, start, duration, ease }
    }

    /// Holds `value` until the tween begins.
    pub fn holding(mut self, value: f64) -> Self {
        self.before = value;
        self
    }

    /// A tween that has already finished at `value`.
    pub fn fixed(value: f64) -> Self {
        Self { before: value, from: value, to: value, start: 0.0, duration: 0.0, ease: Ease::Linear }
    }

    pub fn value(&self, now: f64) -> f64 {
        if self.duration <= 0.0 || now >= self.start + self.duration {
            return self.to;
        }
        if now < self.start {
            return self.before;
        }
        let t = (now - self.start) / self.duration;
        let k = match self.ease {
            Ease::Linear => t,
            Ease::CubicOut => cubic_out(t),
            Ease::BackOut(a) => back_out(t, a),
        };
        self.from + (self.to - self.from) * k
    }

    pub fn is_done(&self, now: f64) -> bool {
        self.duration <= 0.0 || now >= self.start + self.duration
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_response_matches_swiftui() {
        let s = Spring::from_response(0.0, 0.5, 1.0);
        assert!((s.stiffness - (2.0 * PI / 0.5).powi(2)).abs() < 1e-9);
        assert!((s.damping - 4.0 * PI / 0.5).abs() < 1e-9);
    }

    #[test]
    fn springs_settle_on_their_target() {
        let mut s = Spring::from_response(126.0, 0.42, 0.72);
        s.target = 230.0;
        let mut overshoot = false;
        for _ in 0..240 {
            s.step(1.0 / 60.0);
            overshoot |= s.value > 230.5;
        }
        assert!(overshoot, "an under-damped spring overshoots a little");
        assert!(s.is_settled());
        assert_eq!(s.value, 230.0);
    }

    #[test]
    fn eases_start_and_end_where_they_should() {
        assert!(cubic_out(0.0).abs() < 1e-9);
        assert!((cubic_out(1.0) - 1.0).abs() < 1e-9);
        assert!((back_out(1.0, 0.35) - 1.0).abs() < 1e-9);
        assert!((0.0..1.0).any_overshoot(|t| back_out(t, 0.35) > 1.0));
    }

    trait Overshoot {
        fn any_overshoot(self, f: impl Fn(f64) -> bool) -> bool;
    }

    impl Overshoot for std::ops::Range<f64> {
        fn any_overshoot(self, f: impl Fn(f64) -> bool) -> bool {
            (0..100).map(|i| self.start + (self.end - self.start) * i as f64 / 100.0).any(f)
        }
    }
}
