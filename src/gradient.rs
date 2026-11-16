//! Gradients around a path and the lookup of a pixel's colour from `t` and
//! its distance to the path.

use crate::color::Oklch;
use crate::easing::{Curve, Easing};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stop {
    pub dist: f32,
    pub color: Oklch,
    /// How the colour moves from this stop to the next one.
    pub easing: Easing,
}

/// A colour ramp running outward from the path. Stops are sorted by distance.
#[derive(Clone, Debug, PartialEq)]
pub struct Gradient {
    pub t: f32,
    pub stops: Vec<Stop>,
    /// How the colour moves from this gradient to the next one along `t`.
    /// The last gradient of a loop (a whole open path, or one side of a
    /// closed path) eases back round to the first one of that loop.
    pub easing: Easing,
}

/// A [`Gradient`] laid out for per-pixel lookup: stop distances in their own
/// array for a cache-friendly search, easings prepared as [`Curve`]s.
#[derive(Clone, Debug)]
struct Ramp {
    t: f32,
    dists: Box<[f32]>,
    colors: Box<[Oklch]>,
    /// `curves[i]` eases from stop `i` to stop `i + 1`.
    curves: Box<[Curve]>,
    /// Eases from this gradient to the next one along `t`.
    easing: Curve,
}

impl Ramp {
    fn new(g: &Gradient) -> Ramp {
        let n = g.stops.len();
        Ramp {
            t: g.t,
            dists: g.stops.iter().map(|s| s.dist).collect(),
            colors: g.stops.iter().map(|s| s.color).collect(),
            curves: g.stops[..n.saturating_sub(1)]
                .iter()
                .map(|s| Curve::new(s.easing))
                .collect(),
            easing: Curve::new(g.easing),
        }
    }

    /// The stops around distance `d`: `Ok((i, f))` to blend stop `i` into
    /// stop `i + 1` by `f`, or `Err(i)` to take stop `i` as it is.
    #[inline]
    fn locate(&self, d: f32) -> Result<(usize, f32), usize> {
        let s = &self.dists;
        // Number of stops at or before `d`.
        let i = s.partition_point(|&x| x <= d);
        if i == 0 {
            return Err(0);
        }
        if i == s.len() {
            return Err(i - 1);
        }
        let f = self.curves[i - 1].apply((d - s[i - 1]) / (s[i] - s[i - 1]));
        Ok((i - 1, f))
    }

    /// Colour at distance `d` from the path.
    #[inline]
    fn eval(&self, d: f32) -> Oklch {
        match self.locate(d) {
            Ok((i, f)) => self.colors[i].lerp(self.colors[i + 1], f),
            Err(i) => self.colors[i],
        }
    }

    /// Alpha at distance `d` from the path; the same as `self.eval(d).a`.
    #[inline]
    fn eval_alpha(&self, d: f32) -> f32 {
        match self.locate(d) {
            Ok((i, f)) => lerp_alpha(self.colors[i].a, self.colors[i + 1].a, f),
            Err(i) => self.colors[i].a,
        }
    }

    /// Distance beyond which this gradient is fully transparent, if its last
    /// stop is transparent.
    fn extent(&self) -> Option<f32> {
        let last = self.colors.last()?;
        (last.a <= 0.0).then(|| *self.dists.last().unwrap())
    }
}

/// The gradients of one loop of `t`: the whole of `[0, 4)` for an open path,
/// or one side (`[1, 2)` or `[3, 4)`) of a closed path.
#[derive(Clone, Debug, Default)]
pub struct Ring {
    /// Length of the loop in `t`.
    period: f32,
    /// Gradients sorted (stably) by `t`.
    ramps: Vec<Ramp>,
}

impl Ring {
    pub fn new(period: f32, gradients: &[Gradient]) -> Ring {
        let mut ramps: Vec<Ramp> = gradients.iter().map(Ramp::new).collect();
        ramps.sort_by(|a, b| a.t.total_cmp(&b.t));
        Ring { period, ramps }
    }

    pub fn is_empty(&self) -> bool {
        self.ramps.is_empty()
    }

    /// The two gradients next to `t`, and the blend factor between them,
    /// eased by the first one's easing. Needs at least two gradients.
    #[inline]
    fn neighbours(&self, t: f32) -> (&Ramp, &Ramp, f32) {
        let g = &self.ramps;
        let n = g.len();
        let j = g.partition_point(|x| x.t <= t);
        let (prev, prev_t) = if j == 0 {
            (&g[n - 1], g[n - 1].t - self.period)
        } else {
            (&g[j - 1], g[j - 1].t)
        };
        let (next, next_t) = if j == n {
            (&g[0], g[0].t + self.period)
        } else {
            (&g[j], g[j].t)
        };
        let span = next_t - prev_t;
        let f = if span > 0.0 {
            prev.easing.apply((t - prev_t) / span)
        } else {
            0.0
        };
        (prev, next, f)
    }

    /// Colour at position `t` (within this ring) and distance `d`.
    #[inline]
    pub fn eval(&self, t: f32, d: f32) -> Oklch {
        if let [only] = &self.ramps[..] {
            return only.eval(d);
        }
        let (a, b, f) = self.neighbours(t);
        a.eval(d).lerp(b.eval(d), f)
    }

    /// Alpha at position `t` and distance `d`; the same as `self.eval(t, d).a`.
    #[inline]
    pub fn eval_alpha(&self, t: f32, d: f32) -> f32 {
        if let [only] = &self.ramps[..] {
            return only.eval_alpha(d);
        }
        let (a, b, f) = self.neighbours(t);
        lerp_alpha(a.eval_alpha(d), b.eval_alpha(d), f)
    }

    /// The implicit width of this ring alone (0 if it has no gradients).
    pub fn extent(&self) -> Option<f32> {
        let mut m = 0.0f32;
        for g in &self.ramps {
            m = m.max(g.extent()?);
        }
        Some(m)
    }
}

/// Alpha interpolation exactly as in [`Oklch::lerp`].
#[inline]
fn lerp_alpha(a: f32, b: f32, f: f32) -> f32 {
    let r = a + (b - a) * f;
    if r <= 0.0 { 0.0 } else { r }
}

/// All the colour information of a path.
#[derive(Clone, Debug)]
pub enum Paint {
    /// Open path: one ring over `[0, 4)`.
    Open(Ring),
    /// Closed path: left side `[1, 2)` and right side `[3, 4)`. An empty side
    /// is left unpainted.
    Closed { left: Ring, right: Ring },
}

impl Paint {
    pub fn new(closed: bool, gradients: &[Gradient]) -> Paint {
        if closed {
            let (left, right): (Vec<_>, Vec<_>) =
                gradients.iter().cloned().partition(|g| g.t < 2.0);
            Paint::Closed {
                left: Ring::new(1.0, &left),
                right: Ring::new(1.0, &right),
            }
        } else {
            Paint::Open(Ring::new(4.0, gradients))
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Paint::Open(r) => r.is_empty(),
            Paint::Closed { left, right } => left.is_empty() && right.is_empty(),
        }
    }

    /// The implicit width: if every gradient ends transparent, the greatest
    /// distance at which anything is painted.
    pub fn extent(&self) -> Option<f32> {
        match self {
            Paint::Open(r) => r.extent(),
            Paint::Closed { left, right } => Some(left.extent()?.max(right.extent()?)),
        }
    }

    /// For a closed path, the implicit widths of the left and right sides.
    pub fn side_extents(&self) -> Option<(Option<f32>, Option<f32>)> {
        match self {
            Paint::Open(_) => None,
            Paint::Closed { left, right } => Some((left.extent(), right.extent())),
        }
    }

    /// Colour at `t` and distance `d`, or `None` for an unpainted side.
    #[inline]
    pub fn eval(&self, t: f32, d: f32) -> Option<Oklch> {
        let ring = self.ring(t);
        if ring.is_empty() {
            None
        } else {
            Some(ring.eval(t, d))
        }
    }

    /// Alpha at `t` and distance `d`, or `None` for an unpainted side. Skips
    /// the colour work of [`Paint::eval`].
    #[inline]
    pub fn eval_alpha(&self, t: f32, d: f32) -> Option<f32> {
        let ring = self.ring(t);
        if ring.is_empty() {
            None
        } else {
            Some(ring.eval_alpha(t, d))
        }
    }

    #[inline]
    fn ring(&self, t: f32) -> &Ring {
        match self {
            Paint::Open(r) => r,
            Paint::Closed { left, right } => {
                if t < 2.0 {
                    left
                } else {
                    right
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(t: f32, l: f32) -> Gradient {
        Gradient {
            t,
            stops: vec![Stop {
                dist: 0.0,
                color: Oklch::new(l, 0.0, 0.0, 1.0),
                easing: Easing::Linear,
            }],
            easing: Easing::Linear,
        }
    }

    #[test]
    fn single_gradient_is_its_own_neighbour() {
        let r = Ring::new(4.0, &[solid(1.5, 0.3)]);
        for t in [0.0, 1.0, 1.5, 2.0, 3.99] {
            assert_eq!(r.eval(t, 5.0).l, 0.3);
        }
    }

    #[test]
    fn wraps_around() {
        let r = Ring::new(4.0, &[solid(3.0, 0.0), solid(1.0, 1.0)]);
        // Between 3 and 1 (via 4 ≡ 0): t = 0 is half way.
        assert!((r.eval(0.0, 0.0).l - 0.5).abs() < 1e-6);
        assert!((r.eval(3.5, 0.0).l - 0.25).abs() < 1e-6);
        assert!((r.eval(2.0, 0.0).l - 0.5).abs() < 1e-6);
        // Closed side, period 1.
        let r = Ring::new(1.0, &[solid(3.2, 0.0), solid(3.6, 1.0)]);
        // From 3.6 (L 1) to 3.2 + 1 (L 0): a span of 0.6.
        assert!((r.eval(3.0, 0.0).l - 1.0 / 3.0).abs() < 1e-6);
        assert!((r.eval(3.9, 0.0).l - 0.5).abs() < 1e-6);
    }

    #[test]
    fn equal_t_is_a_hard_switch() {
        let r = Ring::new(
            4.0,
            &[
                solid(1.0, 0.0),
                solid(2.0, 0.2),
                solid(2.0, 0.8),
                solid(3.0, 1.0),
            ],
        );
        assert!((r.eval(1.999, 0.0).l - 0.2).abs() < 1e-3);
        assert!((r.eval(2.0, 0.0).l - 0.8).abs() < 1e-6);
        assert!((r.eval(2.001, 0.0).l - 0.8).abs() < 1e-3);
    }

    #[test]
    fn stops() {
        let c = |l| Oklch::new(l, 0.0, 0.0, 1.0);
        let g = Gradient {
            t: 0.0,
            stops: vec![
                Stop {
                    dist: 2.0,
                    color: c(0.0),
                    easing: Easing::Linear,
                },
                Stop {
                    dist: 4.0,
                    color: c(1.0),
                    easing: Easing::Linear,
                },
                Stop {
                    dist: 4.0,
                    color: c(0.5),
                    easing: Easing::Linear,
                },
                Stop {
                    dist: 6.0,
                    color: Oklch::new(0.5, 0.0, 0.0, 0.0),
                    easing: Easing::Linear,
                },
            ],
            easing: Easing::Linear,
        };
        let g = Ramp::new(&g);
        assert_eq!(g.eval(0.0).l, 0.0);
        assert!((g.eval(3.0).l - 0.5).abs() < 1e-6);
        assert_eq!(g.eval(4.0).l, 0.5);
        assert!((g.eval(5.0).a - 0.5).abs() < 1e-6);
        assert_eq!(g.eval(7.0).a, 0.0);
        assert_eq!(g.extent(), Some(6.0));
    }

    #[test]
    fn easing_between_gradients() {
        let eased = |t, l, easing| Gradient {
            easing,
            ..solid(t, l)
        };
        // ease-in from 1 (L 0) to 3 (L 1), linear on the way back via 4 ≡ 0.
        let r = Ring::new(4.0, &[eased(1.0, 0.0, Easing::EASE_IN), solid(3.0, 1.0)]);
        let close = |a: f32, b: f32| (a - b).abs() < 1e-3;
        assert!(close(r.eval(2.0, 0.0).l, 0.3153), "{}", r.eval(2.0, 0.0).l);
        assert!(close(r.eval_alpha(2.0, 0.0), 1.0));
        assert!(close(r.eval(0.0, 0.0).l, 0.5));
        // The last gradient's easing applies across the wrap.
        let steps = Easing::Steps {
            n: 2,
            pos: crate::easing::StepPosition::End,
        };
        let r = Ring::new(4.0, &[solid(1.0, 0.0), eased(3.0, 1.0, steps)]);
        assert_eq!(r.eval(3.5, 0.0).l, 1.0);
        assert_eq!(r.eval(0.5, 0.0).l, 0.5);
    }

    #[test]
    fn closed_sides_ease_on_their_own() {
        let steps = Easing::Steps {
            n: 2,
            pos: crate::easing::StepPosition::End,
        };
        let eased = |t, l| Gradient {
            easing: steps,
            ..solid(t, l)
        };
        // Left side: two gradients, so easing applies, wrapping within [1, 2).
        // Right side: one gradient, whose easing has nothing to blend into.
        let p = Paint::new(true, &[eased(1.2, 0.0), eased(1.6, 1.0), eased(3.5, 0.4)]);
        let l = |t| p.eval(t, 0.0).unwrap().l;
        assert_eq!(l(1.3), 0.0);
        assert_eq!(l(1.5), 0.5);
        // From 1.6 round to 1.2 + 1: half way is 1.9.
        assert_eq!(l(1.75), 1.0);
        assert_eq!(l(1.95), 0.5);
        for t in [3.0, 3.5, 3.9] {
            assert_eq!(l(t), 0.4);
        }
    }

    #[test]
    fn closed_paint_sides() {
        let p = Paint::new(true, &[solid(1.2, 0.4)]);
        assert_eq!(p.eval(1.7, 0.0).unwrap().l, 0.4);
        assert!(p.eval(3.5, 0.0).is_none());
        assert_eq!(p.extent(), None);
        assert_eq!(p.side_extents(), Some((None, Some(0.0))));
    }

    #[test]
    fn alpha_only_matches_full_eval() {
        let c = |a| Oklch::new(0.6, 0.1, 30.0, a);
        let g = |t, a0, a1| Gradient {
            t,
            stops: vec![
                Stop {
                    dist: 0.0,
                    color: c(a0),
                    easing: Easing::Linear,
                },
                Stop {
                    dist: 10.0,
                    color: c(a1),
                    easing: Easing::Linear,
                },
            ],
            easing: Easing::Linear,
        };
        let p = Paint::new(
            false,
            &[g(0.5, 1.0, 0.0), g(2.0, 0.3, 0.8), g(3.0, 0.0, 0.0)],
        );
        for i in 0..40 {
            for d in [0.0, 2.5, 7.0, 12.0] {
                let t = i as f32 * 0.1;
                assert_eq!(
                    p.eval(t, d).unwrap().a,
                    p.eval_alpha(t, d).unwrap(),
                    "t {t} d {d}"
                );
            }
        }
    }

    #[test]
    fn easing_shapes_the_blend() {
        // From (l0, a0) at 0 to (l1, a1) at 10, eased by `easing`.
        let g = |easing, (l0, a0), (l1, a1)| {
            Ramp::new(&Gradient {
                t: 0.0,
                stops: vec![
                    Stop {
                        dist: 0.0,
                        color: Oklch::new(l0, 0.0, 0.0, a0),
                        easing,
                    },
                    Stop {
                        dist: 10.0,
                        color: Oklch::new(l1, 0.0, 0.0, a1),
                        easing: Easing::Linear,
                    },
                ],
                easing: Easing::Linear,
            })
        };
        let close = |a: f32, b: f32| (a - b).abs() < 1e-3;
        // Opaque stops for lightness (lerp is premultiplied), constant
        // lightness for alpha.
        let light = |e| g(e, (0.0, 1.0), (1.0, 1.0));
        let fade = |e| g(e, (0.5, 1.0), (0.5, 0.0));

        assert!(close(light(Easing::Linear).eval(5.0).l, 0.5));
        assert!(close(fade(Easing::Linear).eval_alpha(5.0), 0.5));

        // ease-in at 0.5 ≈ 0.3153.
        let l = light(Easing::EASE_IN);
        assert!(close(l.eval(5.0).l, 0.3153), "{}", l.eval(5.0).l);
        let f = fade(Easing::EASE_IN);
        assert!(close(f.eval_alpha(5.0), 1.0 - 0.3153));
        assert_eq!(f.eval(5.0).a, f.eval_alpha(5.0));

        // steps(4): flat bands, then the next stop's colour exactly at its distance.
        let steps = Easing::Steps {
            n: 4,
            pos: crate::easing::StepPosition::End,
        };
        let s = light(steps);
        assert_eq!(s.eval(1.0).l, 0.0);
        assert_eq!(s.eval(3.0).l, 0.25);
        assert_eq!(s.eval(9.9).l, 0.75);
        assert_eq!(s.eval(10.0).l, 1.0);
        assert_eq!(fade(steps).eval_alpha(3.0), 0.75);
    }
}
