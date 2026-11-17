//! Scene analysis and rendering for the editor.
//!
//! - [`analyze`] parses a scene and describes its diagnostics and every
//!   `path` node: its points (with the source ranges of their coordinates,
//!   so the editor can rewrite them in place) and the Bézier segments
//!   Hobby's algorithm gives them.
//! - [`scene`] parses a scene and scales it, ready for rendering.
//!
//! All source positions returned are UTF-16 code unit offsets, which is what
//! JavaScript strings index by.

use kdl::{KdlDocument, KdlNode};
use miette::{Severity, SourceSpan};
use serde::Serialize;

use szeel::hobby::hobby;
use szeel::parse::{Scene, parse};

/// Maps byte offsets in a UTF-8 string to UTF-16 offsets.
struct Offsets {
    /// `(byte offset, utf16 offset)` at every non-ASCII char boundary.
    marks: Vec<(usize, usize)>,
}

impl Offsets {
    fn new(src: &str) -> Offsets {
        let mut marks = Vec::new();
        let mut extra = 0usize; // utf16 offset minus byte offset, accumulated
        for (b, c) in src.char_indices() {
            let (l8, l16) = (c.len_utf8(), c.len_utf16());
            if l8 != l16 {
                extra += l8 - l16;
                marks.push((b + l8, b + l8 - extra));
            }
        }
        Offsets { marks }
    }

    fn map(&self, byte: usize) -> usize {
        match self.marks.partition_point(|&(b, _)| b <= byte) {
            0 => byte,
            i => {
                let (b, u) = self.marks[i - 1];
                u + (byte - b)
            }
        }
    }

    fn range(&self, s: SourceSpan) -> [usize; 2] {
        [self.map(s.offset()), self.map(s.offset() + s.len())]
    }
}

#[derive(Serialize)]
struct Diagnostic {
    severity: &'static str,
    message: String,
    label: Option<String>,
    help: Option<String>,
    range: [usize; 2],
}

#[derive(Serialize)]
struct Point {
    x: f64,
    y: f64,
    /// Source range of the whole `point` node.
    node: [usize; 2],
    /// Source ranges of the x and y values.
    xr: [usize; 2],
    yr: [usize; 2],
}

#[derive(Serialize)]
struct PathInfo {
    /// Source range of the whole `path` node, children included.
    node: [usize; 2],
    /// Source range of the `path` name.
    name: [usize; 2],
    id: Option<String>,
    closed: bool,
    /// "draw", "mask" or "defs".
    role: &'static str,
    points: Vec<Point>,
    /// Bézier segments, each `[x0, y0, x1, y1, x2, y2, x3, y3]`.
    curves: Vec<[f64; 8]>,
}

#[derive(Serialize)]
pub struct Analysis {
    ok: bool,
    width: u32,
    height: u32,
    diagnostics: Vec<Diagnostic>,
    paths: Vec<PathInfo>,
}

fn severity(s: Severity) -> &'static str {
    match s {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Advice => "info",
    }
}

/// Parses `src` and describes it for the editor.
pub fn analyze(src: &str) -> Analysis {
    let off = Offsets::new(src);
    let mut out = Analysis {
        ok: false,
        width: 0,
        height: 0,
        diagnostics: Vec::new(),
        paths: Vec::new(),
    };
    match parse(src) {
        Err(e) => {
            for d in e.diagnostics {
                out.diagnostics.push(Diagnostic {
                    severity: severity(d.severity),
                    message: d.message.unwrap_or_else(|| "syntax error".into()),
                    label: d.label,
                    help: d.help,
                    range: off.range(d.span),
                });
            }
        }
        Ok(parsed) => {
            for d in parsed.diagnostics {
                out.diagnostics.push(Diagnostic {
                    severity: severity(d.severity),
                    message: d.message,
                    label: d.label,
                    help: d.help,
                    range: off.range(d.span),
                });
            }
            if let Some(scene) = &parsed.scene {
                out.ok = true;
                out.width = scene.width;
                out.height = scene.height;
            }
            // The scene has failed or not; outline whatever paths there are,
            // so the overlay keeps working while the text has semantic errors.
            if let Ok(doc) = KdlDocument::parse_v2(src) {
                if !out.ok {
                    if let Some(img) = doc.get("image") {
                        let num = |k: &str| img.get(k).and_then(|v| v.as_integer()).unwrap_or(0);
                        out.width = num("width").clamp(0, u32::MAX as i128) as u32;
                        out.height = num("height").clamp(0, u32::MAX as i128) as u32;
                    }
                }
                outline(&doc, "draw", &off, &mut out.paths);
            }
        }
    }
    out
}

fn number(v: &kdl::KdlValue) -> Option<f64> {
    v.as_float().or_else(|| v.as_integer().map(|i| i as f64))
}

fn outline(doc: &KdlDocument, role: &'static str, off: &Offsets, out: &mut Vec<PathInfo>) {
    for node in doc.nodes() {
        let name = node.name().value();
        let child_role = match name {
            "mask" => "mask",
            "defs" => "defs",
            _ => role,
        };
        if name == "path" {
            out.push(path_info(node, role, off));
        }
        if let Some(children) = node.children() {
            outline(children, child_role, off, out);
        }
    }
}

fn path_info(node: &KdlNode, role: &'static str, off: &Offsets) -> PathInfo {
    let closed = node.get("closed").and_then(|v| v.as_bool()).unwrap_or(false);
    let id = node.get("id").and_then(|v| v.as_string()).map(str::to_owned);
    let mut points = Vec::new();
    for child in node.children().map(|d| d.nodes()).unwrap_or_default() {
        if child.name().value() != "point" {
            continue;
        }
        let args: Vec<_> = child.entries().iter().filter(|e| e.name().is_none()).collect();
        let [ex, ey] = args.as_slice() else { continue };
        let (Some(x), Some(y)) = (number(ex.value()), number(ey.value())) else {
            continue;
        };
        points.push(Point {
            x,
            y,
            node: off.range(child.span()),
            xr: off.range(ex.span()),
            yr: off.range(ey.span()),
        });
    }
    PathInfo {
        node: off.range(node.span()),
        name: off.range(node.name().span()),
        id,
        closed,
        role,
        curves: curves(&points, closed),
        points,
    }
}

fn curves(points: &[Point], closed: bool) -> Vec<[f64; 8]> {
    let mut pts: Vec<[f64; 2]> = Vec::with_capacity(points.len());
    for p in points {
        let p = [p.x, p.y];
        if pts.last() != Some(&p) && p.iter().all(|c| c.is_finite()) {
            pts.push(p);
        }
    }
    if closed && pts.len() > 1 && pts.first() == pts.last() {
        pts.pop();
    }
    if pts.len() < 2 {
        return Vec::new();
    }
    hobby(&pts, closed)
        .into_iter()
        .map(|[a, b, c, d]| [a[0], a[1], b[0], b[1], c[0], c[1], d[0], d[1]])
        .collect()
}

/// Scales a scene's geometry and gradient distances by `s`.
fn scale(scene: &mut Scene, s: f64) {
    scene.width = ((scene.width as f64 * s).round() as u32).max(1);
    scene.height = ((scene.height as f64 * s).round() as u32).max(1);
    for p in &mut scene.paths {
        for pt in &mut p.points {
            pt[0] *= s;
            pt[1] *= s;
        }
        for g in &mut p.gradients {
            for stop in &mut g.stops {
                stop.dist *= s as f32;
            }
        }
    }
}

/// Parses `src` and scales it by `scale_by`; `None` if it has errors.
pub fn scene(src: &str, scale_by: f64) -> Option<Scene> {
    let mut scene = parse(src).ok()?.scene?;
    if scale_by != 1.0 {
        scale(&mut scene, scale_by);
    }
    Some(scene)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_cover_values() {
        let src = "image width=10 height=10\npath closed=#true {\n    point 1 2.5\n    point 7 8 // c\n    gradient 0 { stop 0 0.5 0.1 0; }\n}\n";
        let a = serde_json::to_value(analyze(src)).unwrap();
        let p = &a["paths"][0];
        let s = |r: &serde_json::Value| {
            &src[r[0].as_u64().unwrap() as usize..r[1].as_u64().unwrap() as usize]
        };
        assert_eq!(s(&p["points"][0]["xr"]), "1");
        assert_eq!(s(&p["points"][0]["yr"]), "2.5");
        assert_eq!(s(&p["points"][1]["yr"]), "8");
        assert_eq!(p["curves"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn utf16_offsets() {
        let o = Offsets::new("aé😀b");
        assert_eq!(o.map(0), 0);
        assert_eq!(o.map(1), 1);
        assert_eq!(o.map(3), 2); // after é (2 bytes, 1 unit)
        assert_eq!(o.map(7), 4); // after 😀 (4 bytes, 2 units)
        assert_eq!(o.map(8), 5);
    }
}
