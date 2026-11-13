//! Flattened path geometry: polyline with cumulative arc length, closest-point
//! queries, the mapping from a closest point to the gradient coordinate `t`,
//! and conservative bounds used to cull segments per block of pixels.

use crate::hobby::Cubic;

/// Maximum distance between a Bézier curve and its flattened polyline, in px.
pub const FLATTEN_TOLERANCE: f64 = 0.05;

/// Segments per culling chunk.
const CHUNK: usize = 16;

/// One polyline segment, prepared for closest-point queries.
#[derive(Clone, Copy, Debug)]
pub struct Seg {
    pub ax: f32,
    pub ay: f32,
    pub dx: f32,
    pub dy: f32,
    /// `1 / |d|²`, or 0 for a degenerate segment.
    pub inv_len2: f32,
    /// Arc length at the segment start.
    pub s0: f32,
    /// Segment length.
    pub len: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Aabb {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl Aabb {
    pub const EMPTY: Aabb = Aabb { x0: f32::INFINITY, y0: f32::INFINITY, x1: f32::NEG_INFINITY, y1: f32::NEG_INFINITY };

    fn add(&mut self, x: f32, y: f32) {
        self.x0 = self.x0.min(x);
        self.y0 = self.y0.min(y);
        self.x1 = self.x1.max(x);
        self.y1 = self.y1.max(y);
    }

    pub fn grow(self, r: f32) -> Aabb {
        Aabb { x0: self.x0 - r, y0: self.y0 - r, x1: self.x1 + r, y1: self.y1 + r }
    }

    pub fn overlaps(&self, o: &Aabb) -> bool {
        self.x0 <= o.x1 && o.x0 <= self.x1 && self.y0 <= o.y1 && o.y0 <= self.y1
    }

    /// Squared distance between two boxes (0 if they overlap).
    #[inline]
    fn dist2(&self, o: &Aabb) -> f32 {
        let dx = (o.x0 - self.x1).max(self.x0 - o.x1).max(0.0);
        let dy = (o.y0 - self.y1).max(self.y0 - o.y1).max(0.0);
        dx * dx + dy * dy
    }
}

#[derive(Clone, Debug)]
struct Chunk {
    bbox: Aabb,
    /// A point on the path inside this chunk (its first vertex).
    px: f32,
    py: f32,
    first: u32,
    end: u32,
}

/// A flattened path.
#[derive(Clone, Debug)]
pub struct Polyline {
    pub closed: bool,
    pub segs: Vec<Seg>,
    /// Unit tangent used to decide the side when the closest point is the
    /// vertex at the start of segment `i` (the bisector of the two segments
    /// meeting there).
    vertex_tangent: Vec<[f32; 2]>,
    chunks: Vec<Chunk>,
    pub length: f32,
    pub bbox: Aabb,
}

/// Where the closest point of a path lies, relative to a pixel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    /// Gradient coordinate `t` in `[0, 4)`.
    pub t: f32,
    /// Distance from the pixel to the path.
    pub dist: f32,
}

impl Polyline {
    /// Flattens a chain of cubic segments. For closed paths the chain must end
    /// where it starts.
    pub fn from_cubics(cubics: &[Cubic], closed: bool) -> Polyline {
        let mut verts: Vec<[f64; 2]> = vec![cubics[0][0]];
        for c in cubics {
            let n = subdivisions(c);
            for i in 1..=n {
                let p = eval(c, i as f64 / n as f64);
                if p != *verts.last().unwrap() {
                    verts.push(p);
                }
            }
        }
        Self::from_vertices(&verts, closed)
    }

    /// Builds the polyline from its vertices. For closed paths the last vertex
    /// must equal the first.
    pub fn from_vertices(verts: &[[f64; 2]], closed: bool) -> Polyline {
        assert!(verts.len() >= 2);
        let mut segs = Vec::with_capacity(verts.len() - 1);
        let mut s = 0.0f64;
        let mut bbox = Aabb::EMPTY;
        for w in verts.windows(2) {
            let (a, b) = (w[0], w[1]);
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let len = dx.hypot(dy);
            let l2 = dx * dx + dy * dy;
            segs.push(Seg {
                ax: a[0] as f32,
                ay: a[1] as f32,
                dx: dx as f32,
                dy: dy as f32,
                inv_len2: if l2 > 0.0 { (1.0 / l2) as f32 } else { 0.0 },
                s0: s as f32,
                len: len as f32,
            });
            s += len;
            bbox.add(a[0] as f32, a[1] as f32);
        }
        let last = verts[verts.len() - 1];
        bbox.add(last[0] as f32, last[1] as f32);

        let unit = |sg: &Seg| {
            let l = sg.len.max(f32::MIN_POSITIVE);
            [sg.dx / l, sg.dy / l]
        };
        let n = segs.len();
        let vertex_tangent = (0..n)
            .map(|i| {
                let next = unit(&segs[i]);
                let prev = if i > 0 {
                    unit(&segs[i - 1])
                } else if closed {
                    unit(&segs[n - 1])
                } else {
                    next
                };
                let (x, y) = (prev[0] + next[0], prev[1] + next[1]);
                let l = x.hypot(y);
                if l > 1e-6 { [x / l, y / l] } else { next }
            })
            .collect();

        let chunks = (0..n)
            .step_by(CHUNK)
            .map(|first| {
                let end = (first + CHUNK).min(n);
                let mut b = Aabb::EMPTY;
                for sg in &segs[first..end] {
                    b.add(sg.ax, sg.ay);
                    b.add(sg.ax + sg.dx, sg.ay + sg.dy);
                }
                Chunk { bbox: b, px: segs[first].ax, py: segs[first].ay, first: first as u32, end: end as u32 }
            })
            .collect();

        Polyline { closed, segs, vertex_tangent, chunks, length: s as f32, bbox }
    }

    /// Finds the closest point among `cands` (segment indices) to `(px, py)`
    /// and maps it to `t` and a distance.
    #[inline]
    pub fn hit(&self, px: f32, py: f32, cands: &[u32]) -> Hit {
        let mut best = f32::INFINITY;
        let mut bi = 0usize;
        let mut bu = 0.0f32;
        for &i in cands {
            let sg = &self.segs[i as usize];
            let (vx, vy) = (px - sg.ax, py - sg.ay);
            let u = ((vx * sg.dx + vy * sg.dy) * sg.inv_len2).clamp(0.0, 1.0);
            let (ex, ey) = (vx - u * sg.dx, vy - u * sg.dy);
            let d2 = ex * ex + ey * ey;
            if d2 < best {
                best = d2;
                bi = i as usize;
                bu = u;
            }
        }
        self.classify(px, py, bi, bu, best.sqrt())
    }

    fn classify(&self, px: f32, py: f32, i: usize, u: f32, dist: f32) -> Hit {
        let n = self.segs.len();
        let sg = &self.segs[i];
        let inv_pi = std::f32::consts::FRAC_1_PI;

        if !self.closed && i == 0 && u <= 0.0 {
            // Start cap: from the right side (t = 0) around the back to the left (t = 1).
            let l = sg.len.max(f32::MIN_POSITIVE);
            let (tx, ty) = (sg.dx / l, sg.dy / l);
            let (vx, vy) = (px - sg.ax, py - sg.ay);
            let back = -(vx * tx + vy * ty);
            let right = vx * -ty + vy * tx;
            let t = (back.atan2(right) * inv_pi).clamp(0.0, 1.0);
            return Hit { t, dist };
        }
        if !self.closed && i == n - 1 && u >= 1.0 {
            // End cap: from the left side (t = 2) around the front to the right (t = 3).
            let l = sg.len.max(f32::MIN_POSITIVE);
            let (tx, ty) = (sg.dx / l, sg.dy / l);
            let (vx, vy) = (px - sg.ax - sg.dx, py - sg.ay - sg.dy);
            let fwd = vx * tx + vy * ty;
            let left = vx * ty - vy * tx;
            let t = 2.0 + (fwd.atan2(left) * inv_pi).clamp(0.0, 1.0);
            return Hit { t, dist };
        }

        // Along a side. Pick the tangent that decides left/right.
        let (tx, ty, cx, cy) = if u <= 0.0 {
            let tg = self.vertex_tangent[i];
            (tg[0], tg[1], sg.ax, sg.ay)
        } else if u >= 1.0 {
            let j = if i + 1 < n { i + 1 } else { 0 };
            let tg = self.vertex_tangent[j];
            (tg[0], tg[1], sg.ax + sg.dx, sg.ay + sg.dy)
        } else {
            (sg.dx, sg.dy, sg.ax, sg.ay)
        };
        // y points down: cross > 0 means the pixel is on the right.
        let cross = tx * (py - cy) - ty * (px - cx);
        let s = (sg.s0 + u * sg.len) / self.length.max(f32::MIN_POSITIVE);
        let s = s.clamp(0.0, 1.0);
        let t = if cross > 0.0 {
            let t = 4.0 - s;
            if t >= 4.0 { 3.0 } else { t }
        } else {
            let t = 1.0 + s;
            if t >= 2.0 && self.closed { 1.0 } else { t }
        };
        Hit { t, dist }
    }

    /// All segment indices whose chunk could hold the closest point for some
    /// pixel centre in `r`. Returns the indices and a lower bound on the
    /// distance from any point of `r` to the path.
    pub fn initial_candidates(&self, r: &Aabb, out: &mut Vec<u32>) -> f32 {
        out.clear();
        let mut upper = f32::INFINITY;
        for c in &self.chunks {
            upper = upper.min(max_corner_dist2(r, c.px, c.py));
        }
        let mut lower = f32::INFINITY;
        for c in &self.chunks {
            let d2 = r.dist2(&c.bbox);
            if d2 <= upper {
                lower = lower.min(d2);
                out.extend(c.first..c.end);
            }
        }
        lower.sqrt()
    }

    /// Keeps the segments from `cands` that could hold the closest point for
    /// some point of `r`. Returns a lower bound on the distance from `r` to the
    /// path.
    ///
    /// With `c` the distance from the centre of `r` to a segment and `h` the
    /// half-diagonal of `r`, every point of `r` is within `c + h` of that
    /// segment and at least `c - h` from it. So a segment can only be closest
    /// somewhere in `r` if `c ≤ min(c) + 2h`.
    pub fn filter_candidates(&self, r: &Aabb, cands: &[u32], out: &mut Vec<u32>) -> f32 {
        out.clear();
        let (cx, cy) = ((r.x0 + r.x1) * 0.5, (r.y0 + r.y1) * 0.5);
        let half_diag = 0.5 * (r.x1 - r.x0).hypot(r.y1 - r.y0);
        let mut min2 = f32::INFINITY;
        for &i in cands {
            min2 = min2.min(seg_dist2(&self.segs[i as usize], cx, cy));
        }
        let min = min2.sqrt();
        // Small slack for rounding.
        let limit = min + 2.0 * half_diag + 1e-3;
        let limit2 = limit * limit;
        for &i in cands {
            if seg_dist2(&self.segs[i as usize], cx, cy) <= limit2 {
                out.push(i);
            }
        }
        (min - half_diag).max(0.0)
    }
}

#[inline]
fn seg_dist2(sg: &Seg, px: f32, py: f32) -> f32 {
    let (vx, vy) = (px - sg.ax, py - sg.ay);
    let u = ((vx * sg.dx + vy * sg.dy) * sg.inv_len2).clamp(0.0, 1.0);
    let (ex, ey) = (vx - u * sg.dx, vy - u * sg.dy);
    ex * ex + ey * ey
}

fn max_corner_dist2(r: &Aabb, px: f32, py: f32) -> f32 {
    let dx = (px - r.x0).abs().max((px - r.x1).abs());
    let dy = (py - r.y0).abs().max((py - r.y1).abs());
    dx * dx + dy * dy
}

fn eval(c: &Cubic, t: f64) -> [f64; 2] {
    let mt = 1.0 - t;
    let (a, b, cc, d) = (mt * mt * mt, 3.0 * mt * mt * t, 3.0 * mt * t * t, t * t * t);
    [
        a * c[0][0] + b * c[1][0] + cc * c[2][0] + d * c[3][0],
        a * c[0][1] + b * c[1][1] + cc * c[2][1] + d * c[3][1],
    ]
}

/// Number of uniform subdivisions keeping the flattening error within
/// tolerance (Wang's formula).
fn subdivisions(c: &Cubic) -> usize {
    let dd = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| {
        let x = p[0] - 2.0 * q[0] + r[0];
        let y = p[1] - 2.0 * q[1] + r[1];
        x.hypot(y)
    };
    let m = dd(c[0], c[1], c[2]).max(dd(c[1], c[2], c[3]));
    let n = (0.75 * m / FLATTEN_TOLERANCE).sqrt().ceil();
    (n as usize).clamp(1, 100_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(p: &Polyline) -> Vec<u32> {
        (0..p.segs.len() as u32).collect()
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    /// Horizontal line from (0,0) to (10,0); y down, so +y is the right side.
    fn line() -> Polyline {
        Polyline::from_vertices(&[[0.0, 0.0], [4.0, 0.0], [10.0, 0.0]], false)
    }

    #[test]
    fn sides_by_arc_length() {
        let p = line();
        let c = all(&p);
        let h = p.hit(5.0, -2.0, &c);
        assert!(close(h.t, 1.5) && close(h.dist, 2.0), "{h:?}");
        let h = p.hit(5.0, 3.0, &c);
        assert!(close(h.t, 3.5) && close(h.dist, 3.0), "{h:?}");
        let h = p.hit(2.0, -1.0, &c);
        assert!(close(h.t, 1.2), "{h:?}");
        let h = p.hit(2.0, 1.0, &c);
        assert!(close(h.t, 3.8), "{h:?}");
    }

    #[test]
    fn caps_linear_in_angle() {
        let p = line();
        let c = all(&p);
        // Straight behind the start: half way round the start cap.
        let h = p.hit(-3.0, 0.0, &c);
        assert!(close(h.t, 0.5) && close(h.dist, 3.0), "{h:?}");
        // 45° from the right side towards the back.
        let h = p.hit(-1.0, 1.0, &c);
        assert!(close(h.t, 0.25), "{h:?}");
        let h = p.hit(-1.0, -1.0, &c);
        assert!(close(h.t, 0.75), "{h:?}");
        // End cap.
        let h = p.hit(13.0, 0.0, &c);
        assert!(close(h.t, 2.5), "{h:?}");
        let h = p.hit(11.0, -1.0, &c);
        assert!(close(h.t, 2.25), "{h:?}");
        let h = p.hit(11.0, 1.0, &c);
        assert!(close(h.t, 2.75), "{h:?}");
    }

    #[test]
    fn closed_sides_wrap() {
        // Square, clockwise on screen (y down): (0,0) → (10,0) → (10,10) → (0,10).
        let p = Polyline::from_vertices(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0], [0.0, 0.0]], true);
        let c = all(&p);
        // Inside is on the right when going clockwise on screen.
        let h = p.hit(5.0, 1.0, &c);
        assert!(close(h.t, 4.0 - 0.125), "{h:?}");
        let h = p.hit(5.0, -1.0, &c);
        assert!(close(h.t, 1.125), "{h:?}");
        // Outside, near the start corner: the closest point is the start vertex.
        let h = p.hit(-1.0, -1.0, &c);
        assert!(close(h.t, 1.0), "{h:?}");
        // Inside, near the end of the loop: the right side runs backwards.
        let h = p.hit(0.5, 2.0, &c);
        assert!(close(h.t, 3.05), "{h:?}");
    }

    #[test]
    fn culling_keeps_the_closest_segment() {
        let cubics = crate::hobby::hobby(&[[10.0, 10.0], [200.0, 40.0], [120.0, 220.0], [30.0, 90.0]], true);
        let p = Polyline::from_cubics(&cubics, true);
        let everything = all(&p);
        let mut c0 = Vec::new();
        let mut c1 = Vec::new();
        for by in 0..8 {
            for bx in 0..8 {
                let r = Aabb {
                    x0: bx as f32 * 32.0 + 0.5,
                    y0: by as f32 * 32.0 + 0.5,
                    x1: bx as f32 * 32.0 + 31.5,
                    y1: by as f32 * 32.0 + 31.5,
                };
                p.initial_candidates(&r, &mut c0);
                p.filter_candidates(&r, &c0, &mut c1);
                for y in 0..32 {
                    for x in 0..32 {
                        let (px, py) = (r.x0 + x as f32, r.y0 + y as f32);
                        let a = p.hit(px, py, &everything);
                        let b = p.hit(px, py, &c1);
                        assert!((a.dist - b.dist).abs() < 1e-3, "{a:?} {b:?}");
                    }
                }
            }
        }
    }
}
