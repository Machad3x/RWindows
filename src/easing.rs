//! Win11 motion: cubic-bezier curves, WinUI durations, a tween.

use std::time::{Duration, Instant};

/// CSS-style cubic bezier with endpoints (0,0) and (1,1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CubicBezier {
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
}

/// Identity curve.
pub const LINEAR: CubicBezier = CubicBezier::new(0.0, 0.0, 1.0, 1.0);
/// WinUI `ControlFastOutSlowInKeySpline` (0,0,0,1): entrances and point-to-point moves.
pub const FAST_OUT_SLOW_IN: CubicBezier = CubicBezier::new(0.0, 0.0, 0.0, 1.0);

/// WinUI `ControlFasterAnimationDuration` (brush transitions, hover).
pub const FASTER: Duration = Duration::from_millis(83);
/// WinUI `ControlFastAnimationDuration`.
pub const FAST: Duration = Duration::from_millis(167);
/// WinUI `ControlNormalAnimationDuration`.
pub const NORMAL: Duration = Duration::from_millis(250);

impl CubicBezier {
    /// Control points P1=(x1,y1), P2=(x2,y2); x values must be in 0..=1.
    pub const fn new(x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
        Self { x1, y1, x2, y2 }
    }

    /// Progress (y) at time fraction `t` (x), `t` clamped to 0..=1.
    pub fn ease(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        let mut s = t;
        // Newton first (fast), bisection if the slope is too flat to converge.
        for _ in 0..8 {
            let err = bezier(s, self.x1, self.x2) - t;
            if err.abs() < 1e-5 {
                return bezier(s, self.y1, self.y2);
            }
            let slope = bezier_slope(s, self.x1, self.x2);
            if slope.abs() < 1e-6 {
                break;
            }
            s = (s - err / slope).clamp(0.0, 1.0);
        }
        let (mut lo, mut hi) = (0.0_f32, 1.0_f32);
        s = t;
        while hi - lo > 1e-5 {
            if bezier(s, self.x1, self.x2) < t {
                lo = s;
            } else {
                hi = s;
            }
            s = (lo + hi) * 0.5;
        }
        bezier(s, self.y1, self.y2)
    }
}

fn bezier(s: f32, a: f32, b: f32) -> f32 {
    let u = 1.0 - s;
    3.0 * a * u * u * s + 3.0 * b * u * s * s + s * s * s
}

fn bezier_slope(s: f32, a: f32, b: f32) -> f32 {
    let u = 1.0 - s;
    3.0 * a * u * u + 6.0 * (b - a) * u * s + 3.0 * (1.0 - b) * s * s
}

/// A value animating from one number to another over time.
#[derive(Clone, Copy, Debug)]
pub struct Tween {
    from: f32,
    to: f32,
    start: Instant,
    duration: Duration,
    curve: CubicBezier,
}

impl Tween {
    /// A tween resting at `value`.
    pub fn idle(value: f32) -> Self {
        Self { from: value, to: value, start: Instant::now(), duration: Duration::ZERO, curve: LINEAR }
    }

    /// Animates from the current value to `to`. No-op if already heading there.
    pub fn retarget(&mut self, to: f32, duration: Duration, curve: CubicBezier) {
        if self.to == to {
            return;
        }
        *self = Self { from: self.value(), to, start: Instant::now(), duration, curve };
    }

    /// Current value.
    pub fn value(&self) -> f32 {
        if self.duration.is_zero() {
            return self.to;
        }
        let t = self.start.elapsed().as_secs_f32() / self.duration.as_secs_f32();
        if t >= 1.0 {
            self.to
        } else {
            self.from + (self.to - self.from) * self.curve.ease(t)
        }
    }

    /// True while frames are still needed.
    pub fn running(&self) -> bool {
        self.start.elapsed() < self.duration
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ease_should_hit_endpoints() {
        assert_eq!((FAST_OUT_SLOW_IN.ease(0.0), FAST_OUT_SLOW_IN.ease(1.0)), (0.0, 1.0));
    }

    #[test]
    fn linear_should_be_identity() {
        assert!([0.1, 0.25, 0.5, 0.9].iter().all(|&t| (LINEAR.ease(t) - t).abs() < 1e-3));
    }

    #[test]
    fn fast_out_slow_in_should_be_monotonic_and_front_loaded() {
        let ys: Vec<f32> = (0..=20).map(|i| FAST_OUT_SLOW_IN.ease(i as f32 / 20.0)).collect();
        assert!(ys.windows(2).all(|w| w[1] >= w[0]) && ys[10] > 0.8);
    }

    #[test]
    fn idle_tween_should_not_run() {
        assert!(!Tween::idle(1.0).running());
    }
}
