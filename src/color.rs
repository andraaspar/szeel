//! OKLCH colours, OKLCH interpolation (CSS Color 4 rules) and conversion to sRGB.

/// A non-premultiplied OKLCH colour with alpha. Hue is in degrees, in `[0, 360)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Oklch {
    pub l: f32,
    pub c: f32,
    pub h: f32,
    pub a: f32,
}

/// Chroma at or below this counts as "no hue" (CSS Color 4 powerless hue).
const ACHROMATIC: f32 = 1e-5;

impl Oklch {
    pub const TRANSPARENT: Oklch = Oklch {
        l: 0.0,
        c: 0.0,
        h: 0.0,
        a: 0.0,
    };

    /// Builds a colour, normalising the hue into `[0, 360)`.
    pub fn new(l: f32, c: f32, h: f32, a: f32) -> Self {
        Oklch {
            l,
            c,
            h: h.rem_euclid(360.0),
            a,
        }
    }

    /// Interpolates from `self` (at `f = 0`) to `other` (at `f = 1`) in OKLCH:
    /// shorter-arc hue, missing hue taken from the other colour, L and C
    /// premultiplied by alpha.
    #[inline]
    pub fn lerp(self, other: Oklch, f: f32) -> Oklch {
        let a = self.a + (other.a - self.a) * f;
        if a <= 0.0 {
            return Oklch::TRANSPARENT;
        }
        let lp = self.l * self.a + (other.l * other.a - self.l * self.a) * f;
        let cp = self.c * self.a + (other.c * other.a - self.c * self.a) * f;

        let (mut h0, mut h1) = (self.h, other.h);
        if self.c <= ACHROMATIC {
            h0 = h1;
        } else if other.c <= ACHROMATIC {
            h1 = h0;
        }
        let mut dh = h1 - h0;
        if dh > 180.0 {
            dh -= 360.0;
        } else if dh < -180.0 {
            dh += 360.0;
        }
        let mut h = h0 + dh * f;
        if h < 0.0 {
            h += 360.0;
        } else if h >= 360.0 {
            h -= 360.0;
        }
        let inv = 1.0 / a;
        Oklch {
            l: lp * inv,
            c: cp * inv,
            h,
            a,
        }
    }

    /// Converts to premultiplied, gamma-encoded sRGB `[r, g, b, a]`, clipping
    /// out-of-gamut colours.
    #[inline]
    pub fn to_srgb_premul(self) -> [f32; 4] {
        let (s, c) = self.h.to_radians().sin_cos();
        let (oa, ob) = (self.c * c, self.c * s);
        let l_ = self.l + 0.396_337_78 * oa + 0.215_803_76 * ob;
        let m_ = self.l - 0.105_561_346 * oa - 0.063_854_17 * ob;
        let s_ = self.l - 0.089_484_18 * oa - 1.291_485_5 * ob;
        let (l, m, s) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
        let r = 4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s;
        let g = -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s;
        let b = -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s;
        let a = self.a;
        [
            srgb_encode(r) * a,
            srgb_encode(g) * a,
            srgb_encode(b) * a,
            a,
        ]
    }
}

const LUT_SIZE: usize = 4096;

static SRGB_LUT: std::sync::LazyLock<Box<[f32; LUT_SIZE + 1]>> = std::sync::LazyLock::new(|| {
    let mut t = Box::new([0.0f32; LUT_SIZE + 1]);
    for (i, v) in t.iter_mut().enumerate() {
        *v = srgb_encode_exact(i as f64 / LUT_SIZE as f64) as f32;
    }
    t
});

/// The exact sRGB transfer function (linear → gamma-encoded).
pub fn srgb_encode_exact(x: f64) -> f64 {
    if x <= 0.003_130_8 {
        12.92 * x
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    }
}

/// sRGB transfer function via a lookup table with linear interpolation.
/// Clips the input to `[0, 1]`.
#[inline]
pub fn srgb_encode(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0) * LUT_SIZE as f32;
    let i = (x as usize).min(LUT_SIZE - 1);
    let f = x - i as f32;
    let lut = &**SRGB_LUT;
    lut[i] + (lut[i + 1] - lut[i]) * f
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() <= eps
    }

    #[test]
    fn white_and_black() {
        let w = Oklch::new(1.0, 0.0, 0.0, 1.0).to_srgb_premul();
        for ch in &w[..3] {
            assert!(close(*ch, 1.0, 1e-3), "{w:?}");
        }
        let k = Oklch::new(0.0, 0.0, 0.0, 1.0).to_srgb_premul();
        for ch in &k[..3] {
            assert!(close(*ch, 0.0, 1e-4), "{k:?}");
        }
    }

    #[test]
    fn known_red() {
        // sRGB red is oklch(0.62796 0.25768 29.2339).
        let r = Oklch::new(0.62796, 0.25768, 29.2339, 1.0).to_srgb_premul();
        assert!(
            close(r[0], 1.0, 2e-3) && close(r[1], 0.0, 2e-3) && close(r[2], 0.0, 2e-3),
            "{r:?}"
        );
    }

    #[test]
    fn lut_matches_exact() {
        for i in 0..=1000 {
            let x = i as f32 / 1000.0;
            assert!(close(
                srgb_encode(x),
                srgb_encode_exact(x as f64) as f32,
                2e-4
            ));
        }
    }

    #[test]
    fn hue_shorter_arc() {
        let a = Oklch::new(0.5, 0.1, 350.0, 1.0);
        let b = Oklch::new(0.5, 0.1, 30.0, 1.0);
        assert!(close(a.lerp(b, 0.5).h, 10.0, 1e-3));
        assert!(close(b.lerp(a, 0.25).h, 20.0, 1e-3));
        let m = a.lerp(b, 0.75).h;
        assert!(close(m, 20.0, 1e-3), "{m}");
    }

    #[test]
    fn achromatic_takes_other_hue() {
        let grey = Oklch::new(0.9, 0.0, 200.0, 1.0);
        let red = Oklch::new(0.6, 0.2, 30.0, 1.0);
        assert!(close(grey.lerp(red, 0.5).h, 30.0, 1e-4));
        assert!(close(red.lerp(grey, 0.5).h, 30.0, 1e-4));
    }

    #[test]
    fn premultiplied_fade_keeps_colour() {
        let red = Oklch::new(0.6, 0.2, 30.0, 1.0);
        let clear = Oklch::new(0.0, 0.0, 0.0, 0.0);
        let m = red.lerp(clear, 0.5);
        assert!(
            close(m.l, 0.6, 1e-6) && close(m.c, 0.2, 1e-6) && close(m.a, 0.5, 1e-6),
            "{m:?}"
        );
        assert_eq!(red.lerp(clear, 1.0), Oklch::TRANSPARENT);
    }
}
