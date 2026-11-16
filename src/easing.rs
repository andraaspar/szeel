//! Easing functions for the transition between two gradient stops, modelled
//! on the CSS `<easing-function>` type.

use std::fmt;

/// Where the jumps of a [`Easing::Steps`] function fall.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepPosition {
    /// The first jump is at the start (CSS `jump-start` / `start`).
    Start,
    /// The last jump is at the end (CSS `jump-end` / `end`).
    End,
    /// No jump at either end (CSS `jump-none`).
    None,
    /// Jumps at both ends (CSS `jump-both`).
    Both,
}

/// Maps progress between two stops (0 to 1) to the colour blend factor.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Easing {
    #[default]
    Linear,
    /// A CSS `cubic-bezier(x1, y1, x2, y2)`; `x1` and `x2` are in [0, 1].
    CubicBezier { x1: f32, y1: f32, x2: f32, y2: f32 },
    /// A CSS `steps(n, position)`.
    Steps { n: u32, pos: StepPosition },
}

impl Easing {
    pub const EASE: Easing = Easing::CubicBezier {
        x1: 0.25,
        y1: 0.1,
        x2: 0.25,
        y2: 1.0,
    };
    pub const EASE_IN: Easing = Easing::CubicBezier {
        x1: 0.42,
        y1: 0.0,
        x2: 1.0,
        y2: 1.0,
    };
    pub const EASE_OUT: Easing = Easing::CubicBezier {
        x1: 0.0,
        y1: 0.0,
        x2: 0.58,
        y2: 1.0,
    };
    pub const EASE_IN_OUT: Easing = Easing::CubicBezier {
        x1: 0.42,
        y1: 0.0,
        x2: 0.58,
        y2: 1.0,
    };

    /// The eased value of progress `x` in [0, 1], clamped to [0, 1] so that a
    /// curve overshooting like `cubic-bezier(0.3, -0.5, 0.7, 1.5)` cannot push
    /// a colour out of range.
    #[inline]
    pub fn apply(self, x: f32) -> f32 {
        match self {
            Easing::Linear => x,
            Easing::CubicBezier { x1, y1, x2, y2 } => {
                cubic_bezier(x1, y1, x2, y2, x).clamp(0.0, 1.0)
            }
            Easing::Steps { n, pos } => steps(n, pos, x),
        }
    }

    /// Parses a CSS easing function, such as `ease-in` or
    /// `cubic-bezier(0.4, 0, 0.2, 1)`.
    pub fn parse(s: &str) -> Result<Easing, String> {
        let s = s.trim();
        let (name, args) = match s.find('(') {
            None => (s, None),
            Some(i) => {
                let Some(inner) = s[i + 1..].strip_suffix(')') else {
                    return Err("missing `)`".into());
                };
                (s[..i].trim(), Some(inner))
            }
        };
        match (name, args) {
            ("linear", None) => Ok(Easing::Linear),
            ("ease", None) => Ok(Easing::EASE),
            ("ease-in", None) => Ok(Easing::EASE_IN),
            ("ease-out", None) => Ok(Easing::EASE_OUT),
            ("ease-in-out", None) => Ok(Easing::EASE_IN_OUT),
            ("step-start", None) => Ok(Easing::Steps {
                n: 1,
                pos: StepPosition::Start,
            }),
            ("step-end", None) => Ok(Easing::Steps {
                n: 1,
                pos: StepPosition::End,
            }),
            ("cubic-bezier", Some(a)) => {
                let v: Vec<&str> = a.split(',').map(str::trim).collect();
                if v.len() != 4 {
                    return Err(format!("`cubic-bezier` takes 4 numbers, not {}", v.len()));
                }
                let mut n = [0.0f32; 4];
                for (o, t) in n.iter_mut().zip(&v) {
                    *o = t
                        .parse::<f32>()
                        .ok()
                        .filter(|x| x.is_finite())
                        .ok_or_else(|| format!("`{t}` is not a number"))?;
                }
                let [x1, y1, x2, y2] = n;
                if !(0.0..=1.0).contains(&x1) || !(0.0..=1.0).contains(&x2) {
                    return Err("the x values of `cubic-bezier` must be from 0 to 1".into());
                }
                Ok(Easing::CubicBezier { x1, y1, x2, y2 })
            }
            ("steps", Some(a)) => {
                let v: Vec<&str> = a.split(',').map(str::trim).collect();
                if v.is_empty() || v.len() > 2 {
                    return Err("`steps` takes a count and an optional position".into());
                }
                let n: u32 = v[0]
                    .parse()
                    .ok()
                    .filter(|&n| n > 0)
                    .ok_or_else(|| format!("`{}` is not a positive whole number", v[0]))?;
                let pos = match v.get(1).copied() {
                    None | Some("jump-end" | "end") => StepPosition::End,
                    Some("jump-start" | "start") => StepPosition::Start,
                    Some("jump-none") => StepPosition::None,
                    Some("jump-both") => StepPosition::Both,
                    Some(p) => {
                        return Err(format!(
                            "unknown step position `{p}`; expected jump-start, jump-end, \
                             jump-none, jump-both, start or end"
                        ));
                    }
                };
                if pos == StepPosition::None && n < 2 {
                    return Err("`steps` with `jump-none` needs at least 2 steps".into());
                }
                Ok(Easing::Steps { n, pos })
            }
            _ => Err(format!("unknown easing function `{s}`")),
        }
    }
}

impl fmt::Display for Easing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Easing::Linear => f.write_str("linear"),
            Easing::CubicBezier { x1, y1, x2, y2 } => {
                write!(f, "cubic-bezier({x1}, {y1}, {x2}, {y2})")
            }
            Easing::Steps { n, pos } => {
                let p = match pos {
                    StepPosition::Start => "jump-start",
                    StepPosition::End => "jump-end",
                    StepPosition::None => "jump-none",
                    StepPosition::Both => "jump-both",
                };
                write!(f, "steps({n}, {p})")
            }
        }
    }
}

/// Number of segments in a [`Curve::Table`].
const TABLE_SEGMENTS: usize = 512;

/// An [`Easing`] prepared for evaluation per pixel: a cubic Bézier is
/// sampled once into a table, so applying it is usually a lookup and a lerp
/// rather than a numerical solve.
#[derive(Clone, Debug)]
pub enum Curve {
    /// Linear and step easings, which are already cheap.
    Exact(Easing),
    /// A cubic Bézier, tabulated.
    Table(Box<BezierTable>),
}

/// A cubic Bézier easing sampled at `TABLE_SEGMENTS + 1` evenly spaced `x`.
#[derive(Clone, Debug)]
pub struct BezierTable {
    easing: Easing,
    /// Per segment, the value at its start and the slope across it (per
    /// segment width). The slope is NaN where interpolating is not accurate
    /// enough (near a vertical tangent or a clamped overshoot); those
    /// segments are solved exactly.
    seg: [[f32; 2]; TABLE_SEGMENTS],
}

/// Largest error accepted from interpolating a [`BezierTable`]: a small
/// fraction of one 8-bit colour level.
const TABLE_TOLERANCE: f32 = 1e-4;

impl Curve {
    pub fn new(e: Easing) -> Curve {
        if !matches!(e, Easing::CubicBezier { .. }) {
            return Curve::Exact(e);
        }
        let n = TABLE_SEGMENTS as f32;
        let mut t = Box::new(BezierTable {
            easing: e,
            seg: [[0.0; 2]; TABLE_SEGMENTS],
        });
        let mut a = e.apply(0.0);
        for (i, seg) in t.seg.iter_mut().enumerate() {
            let b = e.apply((i + 1) as f32 / n);
            let accurate = (1..8).all(|k| {
                let f = k as f32 / 8.0;
                (a + (b - a) * f - e.apply((i as f32 + f) / n)).abs() <= TABLE_TOLERANCE
            });
            *seg = [a, if accurate { b - a } else { f32::NAN }];
            a = b;
        }
        Curve::Table(t)
    }

    /// Like [`Easing::apply`] for `x` in [0, 1].
    #[inline]
    pub fn apply(&self, x: f32) -> f32 {
        match self {
            Curve::Exact(Easing::Linear) => x,
            Curve::Exact(e) => e.apply(x),
            Curve::Table(t) => {
                let p = (x * TABLE_SEGMENTS as f32).clamp(0.0, TABLE_SEGMENTS as f32);
                let i = (p as usize).min(TABLE_SEGMENTS - 1);
                let [a, slope] = t.seg[i];
                if slope.is_nan() {
                    return t.easing.apply(x);
                }
                a + slope * (p - i as f32)
            }
        }
    }
}

/// The names accepted by [`Easing::parse`], for help messages.
pub const EASING_NAMES: &str = "linear, ease, ease-in, ease-out, ease-in-out, \
     cubic-bezier(x1, y1, x2, y2), steps(n[, position]), step-start, step-end";

/// CSS step easing (without the "before flag", which has no meaning here).
fn steps(n: u32, pos: StepPosition, x: f32) -> f32 {
    let n_f = n as f32;
    let mut step = (x * n_f).floor();
    if matches!(pos, StepPosition::Start | StepPosition::Both) {
        step += 1.0;
    }
    let jumps = match pos {
        StepPosition::Start | StepPosition::End => n_f,
        StepPosition::None => n_f - 1.0,
        StepPosition::Both => n_f + 1.0,
    };
    (step.clamp(0.0, jumps)) / jumps
}

/// Solves the CSS cubic Bézier through (0, 0), (x1, y1), (x2, y2), (1, 1) for
/// the `y` at horizontal position `x`.
fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32, x: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    // Polynomial coefficients of B(s) = ((a s + b) s + c) s per axis.
    let cx = 3.0 * x1;
    let bx = 3.0 * (x2 - x1) - cx;
    let ax = 1.0 - cx - bx;
    let cy = 3.0 * y1;
    let by = 3.0 * (y2 - y1) - cy;
    let ay = 1.0 - cy - by;
    let sample_x = |s: f32| ((ax * s + bx) * s + cx) * s;
    let slope_x = |s: f32| (3.0 * ax * s + 2.0 * bx) * s + cx;

    // Newton's method first; it converges in a few steps for typical curves.
    let mut s = x;
    let mut found = false;
    for _ in 0..8 {
        let err = sample_x(s) - x;
        if err.abs() < 1e-6 {
            found = true;
            break;
        }
        let d = slope_x(s);
        if d.abs() < 1e-6 {
            break;
        }
        s -= err / d;
    }
    // Bisection when Newton fails (flat spots); x(s) is monotonic on [0, 1]
    // because x1 and x2 are in [0, 1].
    if !found || !(0.0..=1.0).contains(&s) {
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        s = x;
        for _ in 0..32 {
            let v = sample_x(s);
            if (v - x).abs() < 1e-6 {
                break;
            }
            if v < x {
                lo = s;
            } else {
                hi = s;
            }
            s = 0.5 * (lo + hi);
        }
    }
    ((ay * s + by) * s + cy) * s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn keywords() {
        assert_eq!(Easing::parse("linear"), Ok(Easing::Linear));
        assert_eq!(Easing::parse(" ease-in "), Ok(Easing::EASE_IN));
        assert_eq!(Easing::parse("ease-out"), Ok(Easing::EASE_OUT));
        assert_eq!(Easing::parse("ease-in-out"), Ok(Easing::EASE_IN_OUT));
        assert_eq!(Easing::parse("ease"), Ok(Easing::EASE));
        assert!(Easing::parse("bounce").is_err());
        assert!(Easing::parse("linear(0, 1)").is_err());
    }

    #[test]
    fn bezier_shapes() {
        for e in [
            Easing::EASE,
            Easing::EASE_IN,
            Easing::EASE_OUT,
            Easing::EASE_IN_OUT,
        ] {
            assert_eq!(e.apply(0.0), 0.0);
            assert_eq!(e.apply(1.0), 1.0);
            let mut prev = 0.0;
            for i in 1..=100 {
                let v = e.apply(i as f32 / 100.0);
                assert!(v >= prev - 1e-6, "{e} not monotonic at {i}");
                prev = v;
            }
        }
        assert!(Easing::EASE_IN.apply(0.5) < 0.5);
        assert!(Easing::EASE_OUT.apply(0.5) > 0.5);
        assert!(close(Easing::EASE_IN_OUT.apply(0.5), 0.5));
        // Known values (Chrome): ease-in at 0.5 ≈ 0.3153, ease at 0.5 ≈ 0.8024.
        assert!(close(Easing::EASE_IN.apply(0.5), 0.3153));
        assert!(close(Easing::EASE.apply(0.5), 0.8024));
        // A linear-shaped bezier is the identity.
        let l = Easing::parse("cubic-bezier(0.25, 0.25, 0.75, 0.75)").unwrap();
        for i in 0..=10 {
            let x = i as f32 / 10.0;
            assert!(close(l.apply(x), x));
        }
    }

    #[test]
    fn bezier_parse_and_clamp() {
        assert!(Easing::parse("cubic-bezier(1.5, 0, 0, 1)").is_err());
        assert!(Easing::parse("cubic-bezier(0, 0, 1)").is_err());
        assert!(Easing::parse("cubic-bezier(0, 0, 1, 1").is_err());
        let o = Easing::parse("cubic-bezier(0.3, -1, 0.7, 2)").unwrap();
        for i in 0..=100 {
            let v = o.apply(i as f32 / 100.0);
            assert!((0.0..=1.0).contains(&v));
        }
    }

    #[test]
    fn curve_matches_easing() {
        let max_err = |s: &str| {
            let e = Easing::parse(s).unwrap();
            let c = Curve::new(e);
            (0..=10_000)
                .map(|i| {
                    let x = i as f32 / 10_000.0;
                    (c.apply(x) - e.apply(x)).abs()
                })
                .fold(0.0f32, f32::max)
        };
        for s in [
            "ease",
            "ease-in",
            "ease-out",
            "ease-in-out",
            // Clamped overshoot.
            "cubic-bezier(0.3, -1, 0.7, 2)",
            // Vertical tangents: y ~ cbrt(x) at the start, and in the middle.
            "cubic-bezier(0, 1, 0, 1)",
            "cubic-bezier(1, 0, 0, 1)",
        ] {
            assert!(max_err(s) < 2.0 * TABLE_TOLERANCE, "{s}: {}", max_err(s));
        }
        for s in ["linear", "steps(4)", "steps(3, jump-both)"] {
            assert_eq!(max_err(s), 0.0, "{s}");
        }
    }

    #[test]
    fn steps_positions() {
        let e = |s| Easing::parse(s).unwrap();
        let s = e("steps(4)");
        assert_eq!(s.apply(0.0), 0.0);
        assert_eq!(s.apply(0.3), 0.25);
        assert_eq!(s.apply(0.99), 0.75);
        assert_eq!(s.apply(1.0), 1.0);
        let s = e("steps(4, jump-start)");
        assert_eq!(s.apply(0.0), 0.25);
        assert_eq!(s.apply(0.99), 1.0);
        let s = e("steps(3, jump-none)");
        assert_eq!(s.apply(0.0), 0.0);
        assert_eq!(s.apply(0.5), 0.5);
        assert_eq!(s.apply(0.9), 1.0);
        let s = e("steps(1, jump-both)");
        assert_eq!(s.apply(0.0), 0.5);
        assert_eq!(e("step-start").apply(0.0), 1.0);
        assert_eq!(e("step-end").apply(0.99), 0.0);
        assert!(Easing::parse("steps(0)").is_err());
        assert!(Easing::parse("steps(1, jump-none)").is_err());
        assert!(Easing::parse("steps(2, sideways)").is_err());
    }
}
