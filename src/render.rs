//! Rendering: tiles of premultiplied sRGB pixels, painted path by path from
//! front to back.

use std::io::Write;

use rayon::prelude::*;

use crate::geometry::{Aabb, Polyline};
use crate::gradient::Paint;
use crate::hobby::hobby;
use crate::parse::{PathDef, Scene};

/// Tile edge, in pixels. Tiles are stored contiguously, so each one can be
/// rendered by one thread.
pub const TILE: usize = 32;
const TILE_PX: usize = TILE * TILE;
/// Edge of the blocks inside a tile that get their own segment list.
const BLOCK: usize = 8;

/// A pixel counts as fully opaque from this alpha on: what any path behind
/// it could still add is below half a step of the 8-bit output.
pub const OPAQUE: f32 = 1.0 - 1.0 / 512.0;

/// A path ready to render: flattened geometry and its paint.
pub struct PreparedPath {
    pub poly: Polyline,
    pub paint: Paint,
    /// The implicit width, if the path has one.
    pub extent: Option<f32>,
}

/// Why a path cannot be rendered.
#[derive(Debug, PartialEq)]
pub enum Skip {
    NoGradients,
    Degenerate,
}

impl PreparedPath {
    pub fn new(def: &PathDef) -> Result<PreparedPath, Skip> {
        let paint = Paint::new(def.closed, def.gradients.clone());
        if paint.is_empty() {
            return Err(Skip::NoGradients);
        }
        let mut pts: Vec<[f64; 2]> = Vec::with_capacity(def.points.len());
        for &p in &def.points {
            if pts.last() != Some(&p) {
                pts.push(p);
            }
        }
        if def.closed && pts.len() > 1 && pts.first() == pts.last() {
            pts.pop();
        }
        if pts.len() < 2 {
            return Err(Skip::Degenerate);
        }
        let poly = Polyline::from_cubics(&hobby(&pts, def.closed), def.closed);
        let extent = paint.extent();
        Ok(PreparedPath { poly, paint, extent })
    }
}

/// The image being rendered: premultiplied, gamma-encoded sRGB in `f32`,
/// stored tile by tile.
pub struct Canvas {
    pub width: usize,
    pub height: usize,
    tiles_x: usize,
    tiles_y: usize,
    px: Vec<[f32; 4]>,
    /// Opaque pixels per tile.
    opaque: Vec<u32>,
}

impl Canvas {
    pub fn new(width: usize, height: usize) -> Canvas {
        let tiles_x = width.div_ceil(TILE);
        let tiles_y = height.div_ceil(TILE);
        let n = tiles_x * tiles_y;
        Canvas { width, height, tiles_x, tiles_y, px: vec![[0.0; 4]; n * TILE_PX], opaque: vec![0; n] }
    }

    fn tile_rect(&self, i: usize) -> (usize, usize, usize, usize) {
        let (tx, ty) = (i % self.tiles_x, i / self.tiles_x);
        let (x0, y0) = (tx * TILE, ty * TILE);
        (x0, y0, (x0 + TILE).min(self.width) - x0, (y0 + TILE).min(self.height) - y0)
    }

    pub fn is_opaque(&self) -> bool {
        (0..self.opaque.len()).all(|i| {
            let (_, _, w, h) = self.tile_rect(i);
            self.opaque[i] as usize == w * h
        })
    }

    /// Paints `path` behind everything painted so far.
    pub fn paint(&mut self, path: &PreparedPath) {
        let tiles_x = self.tiles_x;
        let (width, height) = (self.width, self.height);
        let reach = path.extent.map(|e| path.poly.bbox.grow(e));
        self.px
            .par_chunks_mut(TILE_PX)
            .zip(self.opaque.par_iter_mut())
            .enumerate()
            .for_each_init(
                || (Vec::new(), Vec::new()),
                |(cands, block_cands), (i, (tile, opaque))| {
                    let (tx, ty) = (i % tiles_x, i / tiles_x);
                    let (x0, y0) = (tx * TILE, ty * TILE);
                    let (w, h) = ((x0 + TILE).min(width) - x0, (y0 + TILE).min(height) - y0);
                    if *opaque as usize == w * h {
                        return;
                    }
                    let rect = centres(x0, y0, w, h);
                    if let Some(r) = &reach
                        && !r.overlaps(&rect)
                    {
                        return;
                    }
                    let lower = path.poly.initial_candidates(&rect, cands);
                    if path.extent.is_some_and(|e| lower > e) {
                        return;
                    }
                    for by in (0..h).step_by(BLOCK) {
                        for bx in (0..w).step_by(BLOCK) {
                            let (bw, bh) = (BLOCK.min(w - bx), BLOCK.min(h - by));
                            let r = centres(x0 + bx, y0 + by, bw, bh);
                            let lower = path.poly.filter_candidates(&r, cands, block_cands);
                            if path.extent.is_some_and(|e| lower > e) {
                                continue;
                            }
                            for y in by..by + bh {
                                let row = &mut tile[y * TILE..];
                                let py = (y0 + y) as f32 + 0.5;
                                for (x, dst) in row.iter_mut().enumerate().take(bx + bw).skip(bx) {
                                    if dst[3] >= OPAQUE {
                                        continue;
                                    }
                                    let px = (x0 + x) as f32 + 0.5;
                                    let Some(src) = shade(path, px, py, block_cands) else { continue };
                                    let k = 1.0 - dst[3];
                                    for c in 0..4 {
                                        dst[c] += src[c] * k;
                                    }
                                    if dst[3] >= OPAQUE {
                                        *opaque += 1;
                                    }
                                }
                            }
                        }
                    }
                },
            );
    }

    /// Writes the image as an 8-bit RGBA PNG.
    pub fn write_png<W: Write>(&self, out: W) -> Result<(), png::EncodingError> {
        let mut enc = png::Encoder::new(out, self.width as u32, self.height as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        let mut writer = enc.write_header()?;
        let mut stream = writer.stream_writer()?;
        let row_bytes = self.width * 4;
        let mut band = vec![0u8; row_bytes * TILE];
        for ty in 0..self.tiles_y {
            let rows = (self.height - ty * TILE).min(TILE);
            band[..rows * row_bytes].par_chunks_mut(row_bytes).enumerate().for_each(|(y, row)| {
                for (x, out) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                    let tile = ty * self.tiles_x + x / TILE;
                    let p = self.px[tile * TILE_PX + y * TILE + x % TILE];
                    out.copy_from_slice(&to_rgba8(p));
                }
            });
            stream.write_all(&band[..rows * row_bytes])?;
        }
        stream.finish()?;
        Ok(())
    }

    /// The pixel at `(x, y)` as straight 8-bit RGBA.
    pub fn pixel(&self, x: usize, y: usize) -> [u8; 4] {
        let tile = (y / TILE) * self.tiles_x + x / TILE;
        to_rgba8(self.px[tile * TILE_PX + (y % TILE) * TILE + x % TILE])
    }
}

/// The box spanned by the pixel centres of a block.
fn centres(x0: usize, y0: usize, w: usize, h: usize) -> Aabb {
    Aabb { x0: x0 as f32 + 0.5, y0: y0 as f32 + 0.5, x1: (x0 + w) as f32 - 0.5, y1: (y0 + h) as f32 - 0.5 }
}

/// The colour `path` gives the point `(px, py)`, as premultiplied sRGB, or
/// `None` if it leaves it unpainted. `cands` must contain the segments that
/// can be closest to the point.
#[inline]
pub fn shade(path: &PreparedPath, px: f32, py: f32, cands: &[u32]) -> Option<[f32; 4]> {
    let hit = path.poly.hit(px, py, cands);
    if path.extent.is_some_and(|e| hit.dist > e) {
        return None;
    }
    let c = path.paint.eval(hit.t, hit.dist)?;
    if c.a <= 0.0 {
        return None;
    }
    Some(c.to_srgb_premul())
}

fn to_rgba8(p: [f32; 4]) -> [u8; 4] {
    let a = p[3];
    if a <= 0.0 {
        return [0; 4];
    }
    let q = |v: f32| (v * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
    let inv = 1.0 / a;
    [q(p[0] * inv), q(p[1] * inv), q(p[2] * inv), q(a)]
}

/// Renders a whole scene. Paths that cannot be rendered are passed to `skip`
/// with their index.
pub fn render(scene: &Scene, mut skip: impl FnMut(usize, Skip)) -> Canvas {
    let mut canvas = Canvas::new(scene.width as usize, scene.height as usize);
    for (i, def) in scene.paths.iter().enumerate() {
        if canvas.is_opaque() {
            break;
        }
        match PreparedPath::new(def) {
            Ok(p) => canvas.paint(&p),
            Err(e) => skip(i, e),
        }
    }
    canvas
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse;

    fn scene(src: &str) -> Scene {
        parse(src).unwrap().scene.unwrap()
    }

    #[test]
    fn front_path_wins_and_back_fills_rest() {
        // A red stroke of half-width 3 along y = 20, in front of solid blue.
        let s = scene(
            r#"
image width=70 height=40
path {
    point -10 20
    point 80 20
    gradient 0 {
        stop 0 0.62796 0.25768 29.2339
        stop 3 0.62796 0.25768 29.2339
        stop 3 0 0 0 0
    }
}
path {
    point 0 0
    point 1 0
    gradient 0 { stop 0 0.45201 0.31321 264.052; }
}
"#,
        );
        let c = render(&s, |_, _| panic!());
        assert_eq!(c.pixel(35, 20), [255, 0, 0, 255]);
        assert_eq!(c.pixel(35, 17), [255, 0, 0, 255]);
        assert_eq!(c.pixel(35, 16), [0, 0, 255, 255]);
        assert_eq!(c.pixel(0, 39), [0, 0, 255, 255]);
        assert!(c.is_opaque());
    }

    #[test]
    fn semi_transparent_front_composites_over_back() {
        let s = scene(
            r#"
image width=4 height=4
path {
    point 0 0
    point 4 0
    gradient 0 { stop 0 1 0 0 0.5; }
}
path {
    point 0 0
    point 4 0
    gradient 0 { stop 0 0 0 0; }
}
"#,
        );
        let c = render(&s, |_, _| panic!());
        // White at 50% over black: mid grey in gamma-encoded sRGB.
        let p = c.pixel(1, 1);
        assert!((p[0] as i32 - 128).abs() <= 1 && p[3] == 255, "{p:?}");
    }

    #[test]
    fn implicit_width_leaves_rest_transparent() {
        let s = scene(
            r#"
image width=50 height=50
path closed=#true {
    point 25 10
    point 40 25
    point 25 40
    point 10 25
    gradient 1.5 { stop 0 0.5 0.1 120; stop 2 0.5 0.1 120 0; }
}
"#,
        );
        let c = render(&s, |_, _| panic!());
        assert_eq!(c.pixel(0, 0)[3], 0);
        assert_eq!(c.pixel(25, 25)[3], 0);
        // The path runs clockwise on screen, so its left side is outside.
        assert!(c.pixel(25, 9)[3] > 0);
        assert_eq!(c.pixel(25, 11)[3], 0);
    }

    #[test]
    fn skips_unrenderable_paths() {
        let s = scene("image width=4 height=4\npath { point 1 1; point 1 1; gradient 0 { stop 0 0 0 0; } }\npath { point 0 0; point 1 1 }");
        let mut skipped = Vec::new();
        render(&s, |i, e| skipped.push((i, e)));
        assert_eq!(skipped, [(0, Skip::Degenerate), (1, Skip::NoGradients)]);
    }
}
