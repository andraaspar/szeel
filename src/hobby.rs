//! Hobby's algorithm (as in METAFONT/MetaPost) with tension 1 everywhere and
//! curl 1 at the ends of open paths.
//!
//! Notation follows *The METAFONTbook* / *METAFONT: The Program*: for the
//! segment from `z_k` to `z_{k+1}`, `θ_k` is the angle from the chord to the
//! outgoing direction at `z_k` and `φ_{k+1}` the angle from the incoming
//! direction at `z_{k+1}` to the chord. `ψ_k` is the turning angle of the
//! chords at `z_k`, so tangent continuity gives `θ_k + φ_k = -ψ_k`.

pub type P = [f64; 2];

/// One cubic Bézier segment: start, two control points, end.
pub type Cubic = [P; 4];

/// Computes the cubic Bézier segments of a Hobby path through `pts`.
///
/// `pts` must have no two consecutive equal points (for closed paths, the last
/// point must also differ from the first). Open paths give `pts.len() - 1`
/// segments, closed ones `pts.len()`.
pub fn hobby(pts: &[P], closed: bool) -> Vec<Cubic> {
    let n = pts.len();
    assert!(n >= 2, "a path needs at least 2 points");
    let nseg = if closed { n } else { n - 1 };

    // Chord vectors, lengths and angles.
    let delta: Vec<P> = (0..nseg)
        .map(|k| {
            let (a, b) = (pts[k], pts[(k + 1) % n]);
            [b[0] - a[0], b[1] - a[1]]
        })
        .collect();
    let d: Vec<f64> = delta.iter().map(|v| v[0].hypot(v[1])).collect();
    let ang = |v: P| v[1].atan2(v[0]);

    // ψ_k: turning angle at point k (0 at the ends of open paths).
    let psi: Vec<f64> = (0..n)
        .map(|k| {
            if !closed && (k == 0 || k == n - 1) {
                0.0
            } else {
                let prev = delta[(k + nseg - 1) % nseg];
                let next = delta[k % nseg];
                // Angle of `next` relative to `prev`, in (-π, π].
                let cross = prev[0] * next[1] - prev[1] * next[0];
                let dot = prev[0] * next[0] + prev[1] * next[1];
                cross.atan2(dot)
            }
        })
        .collect();

    // Solve for θ_k (and, on open paths, θ_n := -φ_n).
    let theta: Vec<f64> = if closed {
        // Cyclic system, one row per point:
        // θ_{k-1}/d_{k-1} + (2/d_{k-1} + 2/d_k) θ_k + θ_{k+1}/d_k
        //     = -2ψ_k/d_{k-1} - ψ_{k+1}/d_k
        let mut a = vec![0.0; n];
        let mut b = vec![0.0; n];
        let mut c = vec![0.0; n];
        let mut r = vec![0.0; n];
        for k in 0..n {
            let dp = d[(k + n - 1) % n];
            let dk = d[k];
            a[k] = 1.0 / dp;
            b[k] = 2.0 / dp + 2.0 / dk;
            c[k] = 1.0 / dk;
            r[k] = -2.0 * psi[k] / dp - psi[(k + 1) % n] / dk;
        }
        solve_cyclic_tridiagonal(&a, &b, &c, &r)
    } else if n == 2 {
        // A single segment is a straight line.
        vec![0.0, 0.0]
    } else {
        // Open path, unknowns θ_0..θ_{n-1} with θ_{n-1} = -φ_{n-1}.
        // Curl 1 at the start: θ_0 = φ_1, i.e. θ_0 + θ_1 = -ψ_1.
        // Curl 1 at the end:   φ_last = θ_{last-1}, i.e. θ_{last-1} + θ_last = 0.
        let m = n;
        let mut a = vec![0.0; m];
        let mut b = vec![0.0; m];
        let mut c = vec![0.0; m];
        let mut r = vec![0.0; m];
        b[0] = 1.0;
        c[0] = 1.0;
        r[0] = -psi[1];
        for k in 1..m - 1 {
            let (dp, dk) = (d[k - 1], d[k]);
            a[k] = 1.0 / dp;
            b[k] = 2.0 / dp + 2.0 / dk;
            c[k] = 1.0 / dk;
            r[k] = -2.0 * psi[k] / dp - psi[k + 1] / dk;
        }
        a[m - 1] = 1.0;
        b[m - 1] = 1.0;
        solve_tridiagonal(&a, &b, &c, &r)
    };

    (0..nseg)
        .map(|k| {
            let k1 = (k + 1) % n;
            let th = theta[k];
            let ph = -psi[k1] - theta[k1];
            let (z0, z1) = (pts[k], pts[k1]);
            let base = ang(delta[k]);
            let f0 = velocity(th, ph);
            let f1 = velocity(ph, th);
            let (s0, c0) = (base + th).sin_cos();
            let (s1, c1) = (base - ph).sin_cos();
            [
                z0,
                [z0[0] + f0 * d[k] * c0, z0[1] + f0 * d[k] * s0],
                [z1[0] - f1 * d[k] * c1, z1[1] - f1 * d[k] * s1],
                z1,
            ]
        })
        .collect()
}

/// Hobby's velocity function for tension 1: the control point distance divided
/// by the chord length.
fn velocity(theta: f64, phi: f64) -> f64 {
    let (st, ct) = theta.sin_cos();
    let (sp, cp) = phi.sin_cos();
    let sqrt5 = 5f64.sqrt();
    let num = 2.0 + std::f64::consts::SQRT_2 * (st - sp / 16.0) * (sp - st / 16.0) * (ct - cp);
    let den = 3.0 * (1.0 + 0.5 * (sqrt5 - 1.0) * ct + 0.5 * (3.0 - sqrt5) * cp);
    num / den
}

/// Thomas algorithm. `a[0]` and `c[last]` are ignored.
fn solve_tridiagonal(a: &[f64], b: &[f64], c: &[f64], r: &[f64]) -> Vec<f64> {
    let n = b.len();
    let mut cp = vec![0.0; n];
    let mut x = vec![0.0; n];
    let mut beta = b[0];
    x[0] = r[0] / beta;
    for i in 1..n {
        cp[i - 1] = c[i - 1] / beta;
        beta = b[i] - a[i] * cp[i - 1];
        x[i] = (r[i] - a[i] * x[i - 1]) / beta;
    }
    for i in (0..n - 1).rev() {
        x[i] -= cp[i] * x[i + 1];
    }
    x
}

/// Cyclic tridiagonal system: row `i` is
/// `a[i] x[i-1] + b[i] x[i] + c[i] x[i+1] = r[i]`, indices modulo `n`.
fn solve_cyclic_tridiagonal(a: &[f64], b: &[f64], c: &[f64], r: &[f64]) -> Vec<f64> {
    let n = b.len();
    if n <= 3 {
        // Neighbours alias each other; solve densely.
        let mut m = vec![vec![0.0; n + 1]; n];
        for i in 0..n {
            m[i][(i + n - 1) % n] += a[i];
            m[i][i] += b[i];
            m[i][(i + 1) % n] += c[i];
            m[i][n] = r[i];
        }
        return gauss(m);
    }
    // Sherman–Morrison.
    let gamma = -b[0];
    let mut bb = b.to_vec();
    bb[0] -= gamma;
    bb[n - 1] -= c[n - 1] * a[0] / gamma;
    let x = solve_tridiagonal(a, &bb, c, r);
    let mut u = vec![0.0; n];
    u[0] = gamma;
    u[n - 1] = c[n - 1];
    let z = solve_tridiagonal(a, &bb, c, &u);
    let fact = (x[0] + a[0] * x[n - 1] / gamma) / (1.0 + z[0] + a[0] * z[n - 1] / gamma);
    x.iter().zip(&z).map(|(xi, zi)| xi - fact * zi).collect()
}

fn gauss(mut m: Vec<Vec<f64>>) -> Vec<f64> {
    let n = m.len();
    for col in 0..n {
        let piv = (col..n)
            .max_by(|&i, &j| m[i][col].abs().total_cmp(&m[j][col].abs()))
            .unwrap();
        m.swap(col, piv);
        for row in col + 1..n {
            let f = m[row][col] / m[col][col];
            for k in col..=n {
                m[row][k] -= f * m[col][k];
            }
        }
    }
    let mut x = vec![0.0; n];
    for i in (0..n).rev() {
        let s: f64 = (i + 1..n).map(|k| m[i][k] * x[k]).sum();
        x[i] = (m[i][n] - s) / m[i][i];
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: P, b: P, eps: f64) -> bool {
        (a[0] - b[0]).abs() <= eps && (a[1] - b[1]).abs() <= eps
    }

    #[test]
    fn straight_line() {
        let c = hobby(&[[0.0, 0.0], [30.0, 0.0]], false);
        assert_eq!(c.len(), 1);
        assert!(close(c[0][1], [10.0, 0.0], 1e-9));
        assert!(close(c[0][2], [20.0, 0.0], 1e-9));
    }

    #[test]
    fn collinear_points_stay_straight() {
        let c = hobby(&[[0.0, 0.0], [10.0, 0.0], [30.0, 0.0]], false);
        for seg in &c {
            for p in seg {
                assert!(p[1].abs() < 1e-9);
            }
        }
    }

    /// MetaPost: `(1,0)..(0,1)..(-1,0)..(0,-1)..cycle` gives control points at
    /// distance 0.55229 from each point along the tangent, e.g. the first
    /// segment is `(1,0)..controls (1,0.55229) and (0.55229,1)..(0,1)`.
    #[test]
    fn metapost_circle() {
        let pts = [[1.0, 0.0], [0.0, 1.0], [-1.0, 0.0], [0.0, -1.0]];
        let c = hobby(&pts, true);
        assert_eq!(c.len(), 4);
        assert!(close(c[0][1], [1.0, 0.55229], 1e-5), "{:?}", c[0]);
        assert!(close(c[0][2], [0.55229, 1.0], 1e-5), "{:?}", c[0]);
        assert!(close(c[2][1], [-1.0, -0.55229], 1e-5), "{:?}", c[2]);
    }

    /// MetaPost: `(0,0)..(1,1)..(2,0)` with default curl 1 gives
    /// `(0,0)..controls (0,0.55229) and (0.44771,1)..(1,1)..controls
    /// (1.55229,1) and (2,0.55229)..(2,0)` — the end tangents are vertical
    /// (θ_0 = φ_1 = 45°), and the middle tangent is horizontal.
    #[test]
    fn metapost_three_point_arch() {
        let c = hobby(&[[0.0, 0.0], [1.0, 1.0], [2.0, 0.0]], false);
        assert!(close(c[0][1], [0.0, 0.55229], 1e-5), "{:?}", c[0]);
        assert!(close(c[0][2], [0.44771, 1.0], 1e-5), "{:?}", c[0]);
        assert!(close(c[1][1], [1.55229, 1.0], 1e-5), "{:?}", c[1]);
        assert!(close(c[1][2], [2.0, 0.55229], 1e-5), "{:?}", c[1]);
    }

    #[test]
    fn cyclic_solver_matches_dense() {
        let n = 7;
        let a: Vec<f64> = (0..n).map(|i| 1.0 + i as f64 * 0.1).collect();
        let b: Vec<f64> = (0..n).map(|i| 5.0 + i as f64 * 0.3).collect();
        let c: Vec<f64> = (0..n).map(|i| 0.7 + i as f64 * 0.2).collect();
        let r: Vec<f64> = (0..n).map(|i| (i as f64).sin()).collect();
        let x = solve_cyclic_tridiagonal(&a, &b, &c, &r);
        for i in 0..n {
            let lhs = a[i] * x[(i + n - 1) % n] + b[i] * x[i] + c[i] * x[(i + 1) % n];
            assert!((lhs - r[i]).abs() < 1e-12);
        }
    }

    #[test]
    fn closed_two_and_three_points() {
        let c = hobby(&[[0.0, 0.0], [10.0, 0.0]], true);
        assert_eq!(c.len(), 2);
        let c = hobby(&[[0.0, 0.0], [10.0, 0.0], [5.0, 8.0]], true);
        assert_eq!(c.len(), 3);
        for seg in &c {
            for p in seg {
                assert!(p[0].is_finite() && p[1].is_finite());
            }
        }
    }
}
