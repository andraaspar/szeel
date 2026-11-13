//! Gradients around a path and the lookup of a pixel's colour from `t` and
//! its distance to the path.

use crate::color::Oklch;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stop {
    pub dist: f32,
    pub color: Oklch,
}

/// A colour ramp running outward from the path. Stops are sorted by distance.
#[derive(Clone, Debug, PartialEq)]
pub struct Gradient {
    pub t: f32,
    pub stops: Vec<Stop>,
}

impl Gradient {
    /// Colour at distance `d` from the path.
    #[inline]
    pub fn eval(&self, d: f32) -> Oklch {
        let s = &self.stops;
        // Number of stops at or before `d`.
        let i = s.partition_point(|st| st.dist <= d);
        if i == 0 {
            return s[0].color;
        }
        if i == s.len() {
            return s[i - 1].color;
        }
        let (a, b) = (&s[i - 1], &s[i]);
        let f = (d - a.dist) / (b.dist - a.dist);
        a.color.lerp(b.color, f)
    }

    /// Distance beyond which this gradient is fully transparent, if its last
    /// stop is transparent.
    pub fn extent(&self) -> Option<f32> {
        let last = self.stops.last()?;
        (last.color.a <= 0.0).then_some(last.dist)
    }
}

/// The gradients of one loop of `t`: the whole of `[0, 4)` for an open path,
/// or one side (`[1, 2)` or `[3, 4)`) of a closed path.
#[derive(Clone, Debug, Default)]
pub struct Ring {
    /// Start of the loop in `t`.
    pub start: f32,
    /// Length of the loop in `t`.
    pub period: f32,
    /// Gradients sorted (stably) by `t`.
    pub gradients: Vec<Gradient>,
}

impl Ring {
    pub fn new(start: f32, period: f32, mut gradients: Vec<Gradient>) -> Ring {
        gradients.sort_by(|a, b| a.t.total_cmp(&b.t));
        Ring { start, period, gradients }
    }

    /// The two gradients next to `t`, and the blend factor between them.
    #[inline]
    pub fn neighbours(&self, t: f32) -> (&Gradient, &Gradient, f32) {
        let g = &self.gradients;
        let n = g.len();
        let j = g.partition_point(|x| x.t <= t);
        let (prev, prev_t) = if j == 0 { (&g[n - 1], g[n - 1].t - self.period) } else { (&g[j - 1], g[j - 1].t) };
        let (next, next_t) = if j == n { (&g[0], g[0].t + self.period) } else { (&g[j], g[j].t) };
        let span = next_t - prev_t;
        let f = if span > 0.0 { (t - prev_t) / span } else { 0.0 };
        (prev, next, f)
    }

    /// Colour at position `t` (within this ring) and distance `d`.
    #[inline]
    pub fn eval(&self, t: f32, d: f32) -> Oklch {
        let (a, b, f) = self.neighbours(t);
        if std::ptr::eq(a, b) {
            return a.eval(d);
        }
        a.eval(d).lerp(b.eval(d), f)
    }
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
    pub fn new(closed: bool, gradients: Vec<Gradient>) -> Paint {
        if closed {
            let (left, right): (Vec<_>, Vec<_>) = gradients.into_iter().partition(|g| g.t < 2.0);
            Paint::Closed { left: Ring::new(1.0, 1.0, left), right: Ring::new(3.0, 1.0, right) }
        } else {
            Paint::Open(Ring::new(0.0, 4.0, gradients))
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Paint::Open(r) => r.gradients.is_empty(),
            Paint::Closed { left, right } => left.gradients.is_empty() && right.gradients.is_empty(),
        }
    }

    fn gradients(&self) -> impl Iterator<Item = &Gradient> {
        let (a, b) = match self {
            Paint::Open(r) => (&r.gradients, None),
            Paint::Closed { left, right } => (&left.gradients, Some(&right.gradients)),
        };
        a.iter().chain(b.into_iter().flatten())
    }

    /// The implicit width: if every gradient ends transparent, the greatest
    /// distance at which anything is painted.
    pub fn extent(&self) -> Option<f32> {
        let mut m = 0.0f32;
        for g in self.gradients() {
            m = m.max(g.extent()?);
        }
        Some(m)
    }

    /// Colour at `t` and distance `d`, or `None` for an unpainted side.
    #[inline]
    pub fn eval(&self, t: f32, d: f32) -> Option<Oklch> {
        let ring = match self {
            Paint::Open(r) => r,
            Paint::Closed { left, right } => {
                if t < 2.0 {
                    left
                } else {
                    right
                }
            }
        };
        if ring.gradients.is_empty() { None } else { Some(ring.eval(t, d)) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(t: f32, l: f32) -> Gradient {
        Gradient { t, stops: vec![Stop { dist: 0.0, color: Oklch::new(l, 0.0, 0.0, 1.0) }] }
    }

    #[test]
    fn single_gradient_is_its_own_neighbour() {
        let r = Ring::new(0.0, 4.0, vec![solid(1.5, 0.3)]);
        for t in [0.0, 1.0, 1.5, 2.0, 3.99] {
            assert_eq!(r.eval(t, 5.0).l, 0.3);
        }
    }

    #[test]
    fn wraps_around() {
        let r = Ring::new(0.0, 4.0, vec![solid(3.0, 0.0), solid(1.0, 1.0)]);
        // Between 3 and 1 (via 4 ≡ 0): t = 0 is half way.
        assert!((r.eval(0.0, 0.0).l - 0.5).abs() < 1e-6);
        assert!((r.eval(3.5, 0.0).l - 0.25).abs() < 1e-6);
        assert!((r.eval(2.0, 0.0).l - 0.5).abs() < 1e-6);
        // Closed side, period 1.
        let r = Ring::new(3.0, 1.0, vec![solid(3.2, 0.0), solid(3.6, 1.0)]);
        // From 3.6 (L 1) to 3.2 + 1 (L 0): a span of 0.6.
        assert!((r.eval(3.0, 0.0).l - 1.0 / 3.0).abs() < 1e-6);
        assert!((r.eval(3.9, 0.0).l - 0.5).abs() < 1e-6);
    }

    #[test]
    fn equal_t_is_a_hard_switch() {
        let r = Ring::new(0.0, 4.0, vec![solid(1.0, 0.0), solid(2.0, 0.2), solid(2.0, 0.8), solid(3.0, 1.0)]);
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
                Stop { dist: 2.0, color: c(0.0) },
                Stop { dist: 4.0, color: c(1.0) },
                Stop { dist: 4.0, color: c(0.5) },
                Stop { dist: 6.0, color: Oklch::new(0.5, 0.0, 0.0, 0.0) },
            ],
        };
        assert_eq!(g.eval(0.0).l, 0.0);
        assert!((g.eval(3.0).l - 0.5).abs() < 1e-6);
        assert_eq!(g.eval(4.0).l, 0.5);
        assert!((g.eval(5.0).a - 0.5).abs() < 1e-6);
        assert_eq!(g.eval(7.0).a, 0.0);
        assert_eq!(g.extent(), Some(6.0));
    }

    #[test]
    fn closed_paint_sides() {
        let p = Paint::new(true, vec![solid(1.2, 0.4)]);
        assert_eq!(p.eval(1.7, 0.0).unwrap().l, 0.4);
        assert!(p.eval(3.5, 0.0).is_none());
        assert_eq!(p.extent(), None);
    }
}
