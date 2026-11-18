//! Rendering. The image is split into tiles of premultiplied sRGB pixels, and
//! each tile walks the scene tree front to back, compositing every element
//! behind what is already there ("destination-over").

use std::collections::HashMap;
use std::io::Write;
use std::sync::OnceLock;
#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

use rayon::prelude::*;

use crate::geometry::{Aabb, Polyline};
use crate::gradient::Paint;
use crate::hobby::Cubic;
use crate::parse::{GroupId, Item, MaskId, PathDef, PathId, Scene, Target};

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

impl PreparedPath {
    fn new(def: &PathDef, cubics: &[Cubic]) -> PreparedPath {
        let paint = Paint::new(def.closed, &def.gradients);
        let poly = Polyline::from_cubics(cubics, def.closed);
        let extent = paint.extent();
        PreparedPath {
            poly,
            paint,
            extent,
        }
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
}

impl Canvas {
    pub fn new(width: usize, height: usize) -> Canvas {
        let tiles_x = width.div_ceil(TILE);
        let tiles_y = height.div_ceil(TILE);
        Canvas {
            width,
            height,
            tiles_x,
            tiles_y,
            px: vec![[0.0; 4]; tiles_x * tiles_y * TILE_PX],
        }
    }

    /// Whether every pixel is fully opaque.
    pub fn is_opaque(&self) -> bool {
        (0..self.height).all(|y| (0..self.width).all(|x| self.px[self.index(x, y)][3] >= OPAQUE))
    }

    fn index(&self, x: usize, y: usize) -> usize {
        ((y / TILE) * self.tiles_x + x / TILE) * TILE_PX + (y % TILE) * TILE + x % TILE
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
            band[..rows * row_bytes]
                .par_chunks_mut(row_bytes)
                .enumerate()
                .for_each(|(y, row)| {
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
        to_rgba8(self.px[self.index(x, y)])
    }
}

/// The box spanned by the pixel centres of a block.
fn centres(x0: usize, y0: usize, w: usize, h: usize) -> Aabb {
    Aabb {
        x0: x0 as f32 + 0.5,
        y0: y0 as f32 + 0.5,
        x1: (x0 + w) as f32 - 0.5,
        y1: (y0 + h) as f32 - 0.5,
    }
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

/// The alpha [`shade`] would give, without working out the colour.
#[inline]
fn shade_alpha(path: &PreparedPath, px: f32, py: f32, cands: &[u32]) -> Option<f32> {
    let hit = path.poly.hit(px, py, cands);
    if path.extent.is_some_and(|e| hit.dist > e) {
        return None;
    }
    let a = path.paint.eval_alpha(hit.t, hit.dist)?;
    (a > 0.0).then_some(a)
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

/// Renders a whole scene.
pub fn render(scene: &Scene) -> Canvas {
    render_prepared(&Prepared::new(scene))
}

fn render_prepared(prep: &Prepared) -> Canvas {
    let scene = prep.scene;
    let mut canvas = Canvas::new(scene.width as usize, scene.height as usize);
    let (tiles_x, width, height) = (canvas.tiles_x, canvas.width, canvas.height);
    canvas.px.par_chunks_mut(TILE_PX).enumerate().for_each_init(
        || Walker::new(prep),
        |w, (i, tile)| {
            w.start(TileInfo::new(i, tiles_x, width, height));
            let tile: &mut Colors = tile.try_into().unwrap();
            let inside = w.tile.inside;
            w.draw_list::<Color>(&prep.root, tile, &inside);
        },
    );
    canvas
}

// ---------------------------------------------------------------------------
// Prepared scene

/// Where an element can paint: nowhere outside a box, or anywhere.
#[derive(Clone, Copy, Debug)]
enum Reach {
    All,
    Box(Aabb),
}

impl Reach {
    const NONE: Reach = Reach::Box(Aabb::EMPTY);

    fn union(self, o: Reach) -> Reach {
        match (self, o) {
            (Reach::Box(a), Reach::Box(b)) => Reach::Box(a.union(b)),
            _ => Reach::All,
        }
    }

    fn intersect(self, o: Reach) -> Reach {
        match (self, o) {
            (Reach::All, x) | (x, Reach::All) => x,
            (Reach::Box(a), Reach::Box(b)) => Reach::Box(a.intersect(b)),
        }
    }

    fn overlaps(&self, r: &Aabb) -> bool {
        match self {
            Reach::All => true,
            Reach::Box(b) => b.overlaps(r),
        }
    }
}

/// An entry in a prepared list of things to draw. Unmasked groups that are
/// not reused are inlined, so they never appear here.
#[derive(Debug)]
enum Node {
    Path(PathId),
    Group(GroupId),
    /// A `use` with a mask of its own: the node, masked as a group would be.
    Masked {
        node: Box<Node>,
        mask: MaskId,
    },
}

struct PPath {
    /// The Bézier segments, if computed up front to find the reach.
    cubics: Option<Vec<Cubic>>,
    /// Flattened geometry, made on the first tile that needs it.
    geom: OnceLock<Option<PreparedPath>>,
    mask: Option<MaskId>,
    reach: Reach,
}

impl PPath {
    /// Prepares what is needed for the reach of the path's paint.
    fn new(def: &PathDef) -> PPath {
        let paint = Paint::new(def.closed, &def.gradients);
        let none = PPath {
            cubics: None,
            geom: OnceLock::new(),
            mask: None,
            reach: Reach::NONE,
        };
        if paint.is_empty() {
            return none;
        }
        let Some(cubics) = def.cubics() else {
            return none;
        };
        if let Some(e) = paint.extent() {
            // The curve lies inside the hull of its control points.
            let mut b = Aabb::EMPTY;
            for p in cubics.iter().flatten() {
                b.add(p[0] as f32, p[1] as f32);
            }
            return PPath {
                cubics: Some(cubics),
                reach: Reach::Box(b.grow(e)),
                ..none
            };
        }
        if let Some((left, right)) = paint.side_extents()
            && (left.is_some() || right.is_some())
        {
            // A filled shape: if the loop is simple, a pixel outside its bbox
            // always finds its closest point on the outer side.
            let geom = PreparedPath::new(def, &cubics);
            let area = geom.poly.signed_area2();
            let outer = if area > 0.0 {
                left
            } else if area < 0.0 {
                right
            } else {
                None
            };
            let reach = match outer {
                Some(e) if geom.poly.is_simple() => Reach::Box(geom.poly.bbox.grow(e)),
                _ => Reach::All,
            };
            return PPath {
                geom: OnceLock::from(Some(geom)),
                reach,
                ..none
            };
        }
        PPath {
            reach: Reach::All,
            ..none
        }
    }
}

struct PGroup {
    items: Vec<Node>,
    mask: Option<MaskId>,
    reach: Reach,
}

struct PMask {
    invert: bool,
    items: Vec<Node>,
    mask: Option<MaskId>,
    reach: Reach,
    /// Drawn more than once per tile, so its alpha is cached per tile.
    shared: bool,
}

#[cfg(test)]
#[derive(Default, Debug)]
struct Stats {
    /// Paths flattened.
    prepared: AtomicU64,
    /// Pixels shaded.
    shaded: AtomicU64,
    /// Masks rendered into a tile (cache hits not counted).
    mask_renders: AtomicU64,
}

/// The scene compiled for rendering, with the same tables as [`Scene`].
struct Prepared<'s> {
    scene: &'s Scene,
    paths: Vec<PPath>,
    groups: Vec<PGroup>,
    masks: Vec<PMask>,
    root: Vec<Node>,
    #[cfg(test)]
    stats: Stats,
}

impl<'s> Prepared<'s> {
    fn new(scene: &'s Scene) -> Prepared<'s> {
        // Masks that only `use` one element are the same wherever they appear.
        let mut seen: HashMap<(Target, bool), MaskId> = HashMap::new();
        let canon: Vec<MaskId> = (scene.masks.iter().enumerate())
            .map(|(i, m)| match m.items.as_slice() {
                [Item::Use { target, mask: None }] if m.mask.is_none() => {
                    *seen.entry((*target, m.invert)).or_insert(i as MaskId)
                }
                _ => i as MaskId,
            })
            .collect();
        let cm = |m: Option<MaskId>| m.map(|m| canon[m as usize]);

        let mut paths: Vec<PPath> = scene.paths.iter().map(PPath::new).collect();
        let mut reach = ReachMemo {
            scene,
            own: paths.iter().map(|p| p.reach).collect(),
            paths: vec![None; scene.paths.len()],
            groups: vec![None; scene.groups.len()],
            masks: vec![None; scene.masks.len()],
        };
        for (i, p) in paths.iter_mut().enumerate() {
            p.mask = cm(scene.paths[i].mask);
            p.reach = reach.path(i as PathId);
        }

        // How often each element is drawn per walk (counting up to 2).
        let mut count = Counter {
            scene,
            canon: &canon,
            paths: vec![0; scene.paths.len()],
            groups: vec![0; scene.groups.len()],
            masks: vec![0; scene.masks.len()],
        };
        for &it in &scene.root {
            count.item(it);
        }

        // How many places refer to each group.
        let mut refs = vec![0u32; scene.groups.len()];
        let lists = scene
            .groups
            .iter()
            .map(|g| &g.items)
            .chain(scene.masks.iter().map(|m| &m.items));
        for it in lists.flatten().chain(&scene.root) {
            match *it {
                Item::Group(g)
                | Item::Use {
                    target: Target::Group(g),
                    ..
                } => refs[g as usize] += 1,
                _ => {}
            }
        }

        let mut groups: Vec<PGroup> = (scene.groups.iter().enumerate())
            .map(|(i, g)| PGroup {
                items: Vec::new(),
                mask: cm(g.mask),
                reach: reach.group(i as GroupId),
            })
            .collect();
        // A group's children are completed before it, so they have lower ids.
        let build = |items: &[Item], groups: &mut [PGroup]| {
            let mut out = Vec::with_capacity(items.len());
            for &it in items {
                match it {
                    Item::Path(p) => out.push(Node::Path(p)),
                    Item::Group(g)
                        if scene.groups[g as usize].mask.is_none() && refs[g as usize] == 1 =>
                    {
                        out.append(&mut groups[g as usize].items)
                    }
                    Item::Group(g) => out.push(Node::Group(g)),
                    Item::Use { target, mask } => {
                        let n = match target {
                            Target::Path(p) => Node::Path(p),
                            Target::Group(g) => Node::Group(g),
                        };
                        out.push(match cm(mask) {
                            Some(mask) => Node::Masked {
                                node: Box::new(n),
                                mask,
                            },
                            None => n,
                        });
                    }
                }
            }
            out
        };
        for (i, g) in scene.groups.iter().enumerate() {
            groups[i].items = build(&g.items, &mut groups);
        }
        let masks: Vec<PMask> = (scene.masks.iter().enumerate())
            .map(|(i, m)| PMask {
                invert: m.invert,
                items: build(&m.items, &mut groups),
                mask: cm(m.mask),
                reach: reach.mask(i as MaskId),
                shared: count.masks[i] >= 2,
            })
            .collect();
        let root = build(&scene.root, &mut groups);

        #[cfg(test)]
        let stats = Stats {
            prepared: AtomicU64::new(paths.iter().filter(|p| p.geom.get().is_some()).count() as u64),
            ..Stats::default()
        };
        Prepared {
            scene,
            paths,
            groups,
            masks,
            root,
            #[cfg(test)]
            stats,
        }
    }

    /// The geometry of a path, flattened on first use.
    fn geom(&self, p: PathId) -> Option<&PreparedPath> {
        let pp = &self.paths[p as usize];
        pp.geom
            .get_or_init(|| {
                let def = &self.scene.paths[p as usize];
                let cubics = match &pp.cubics {
                    Some(c) => c,
                    None if def.gradients.is_empty() => return None,
                    None => &def.cubics()?,
                };
                #[cfg(test)]
                self.stats.prepared.fetch_add(1, Relaxed);
                Some(PreparedPath::new(def, cubics))
            })
            .as_ref()
    }

    fn node_reach(&self, n: &Node) -> Reach {
        match n {
            Node::Path(p) => self.paths[*p as usize].reach,
            Node::Group(g) => self.groups[*g as usize].reach,
            Node::Masked { node, mask } => self.node_reach(node).intersect(self.clip(*mask)),
        }
    }

    /// How a mask limits the reach of its element.
    fn clip(&self, m: MaskId) -> Reach {
        let pm = &self.masks[m as usize];
        if pm.invert { Reach::All } else { pm.reach }
    }
}

/// Works out reaches, each element once.
struct ReachMemo<'s> {
    scene: &'s Scene,
    /// Reach of each path's own paint.
    own: Vec<Reach>,
    paths: Vec<Option<Reach>>,
    groups: Vec<Option<Reach>>,
    masks: Vec<Option<Reach>>,
}

impl ReachMemo<'_> {
    fn path(&mut self, p: PathId) -> Reach {
        if let Some(r) = self.paths[p as usize] {
            return r;
        }
        let r = self.own[p as usize].intersect(self.clip(self.scene.paths[p as usize].mask));
        self.paths[p as usize] = Some(r);
        r
    }

    fn group(&mut self, g: GroupId) -> Reach {
        if let Some(r) = self.groups[g as usize] {
            return r;
        }
        let def = &self.scene.groups[g as usize];
        let r = self.list(&def.items).intersect(self.clip(def.mask));
        self.groups[g as usize] = Some(r);
        r
    }

    fn mask(&mut self, m: MaskId) -> Reach {
        if let Some(r) = self.masks[m as usize] {
            return r;
        }
        let def = &self.scene.masks[m as usize];
        let r = self.list(&def.items).intersect(self.clip(def.mask));
        self.masks[m as usize] = Some(r);
        r
    }

    fn clip(&mut self, m: Option<MaskId>) -> Reach {
        match m {
            Some(m) if !self.scene.masks[m as usize].invert => self.mask(m),
            _ => Reach::All,
        }
    }

    fn list(&mut self, items: &[Item]) -> Reach {
        items.iter().fold(Reach::NONE, |acc, &it| {
            let r = match it {
                Item::Path(p) => self.path(p),
                Item::Group(g) => self.group(g),
                Item::Use { target, mask } => {
                    let t = match target {
                        Target::Path(p) => self.path(p),
                        Target::Group(g) => self.group(g),
                    };
                    t.intersect(self.clip(mask))
                }
            };
            acc.union(r)
        })
    }
}

/// Counts how often each element is drawn per walk, up to 2. Each element is
/// entered at most twice, so this is linear in the scene size.
struct Counter<'s> {
    scene: &'s Scene,
    canon: &'s [MaskId],
    paths: Vec<u8>,
    groups: Vec<u8>,
    masks: Vec<u8>,
}

impl Counter<'_> {
    fn item(&mut self, it: Item) {
        match it {
            Item::Path(p) => self.path(p),
            Item::Group(g) => self.group(g),
            Item::Use { target, mask } => {
                match target {
                    Target::Path(p) => self.path(p),
                    Target::Group(g) => self.group(g),
                }
                self.mask(mask);
            }
        }
    }

    fn path(&mut self, p: PathId) {
        let c = &mut self.paths[p as usize];
        if *c < 2 {
            *c += 1;
            self.mask(self.scene.paths[p as usize].mask);
        }
    }

    fn group(&mut self, g: GroupId) {
        let c = &mut self.groups[g as usize];
        if *c < 2 {
            *c += 1;
            let def = &self.scene.groups[g as usize];
            def.items.iter().for_each(|&it| self.item(it));
            self.mask(def.mask);
        }
    }

    fn mask(&mut self, m: Option<MaskId>) {
        let Some(m) = m else { return };
        let m = self.canon[m as usize] as usize;
        if self.masks[m] < 2 {
            self.masks[m] += 1;
            let def = &self.scene.masks[m];
            def.items.iter().for_each(|&it| self.item(it));
            self.mask(def.mask);
        }
    }
}

// ---------------------------------------------------------------------------
// Walking one tile

/// A set of pixels of a tile: bit `x` of row `y`.
type Live = [u32; TILE];
type Alphas = [f32; TILE_PX];
type Colors = [[f32; 4]; TILE_PX];

fn is_empty(l: &Live) -> bool {
    l.iter().all(|&r| r == 0)
}

/// Calls `f` with the index of each pixel in `l`.
#[inline]
fn for_each(l: &Live, mut f: impl FnMut(usize)) {
    for (y, &row) in l.iter().enumerate() {
        let mut bits = row;
        while bits != 0 {
            let x = bits.trailing_zeros() as usize;
            bits &= bits - 1;
            f(y * TILE + x);
        }
    }
}

/// Removes the pixels for which `keep` is false.
#[inline]
fn retain(l: &mut Live, mut keep: impl FnMut(usize) -> bool) {
    for (y, row) in l.iter_mut().enumerate() {
        let mut bits = *row;
        while bits != 0 {
            let x = bits.trailing_zeros() as usize;
            bits &= bits - 1;
            if !keep(y * TILE + x) {
                *row &= !(1 << x);
            }
        }
    }
}

/// What is rendered into a layer: colours, or (inside masks) alpha alone.
trait Mode {
    type Px: Copy;
    const CLEAR: Self::Px;
    fn alpha(p: &Self::Px) -> f32;
    fn shade(path: &PreparedPath, px: f32, py: f32, cands: &[u32]) -> Option<Self::Px>;
    /// `dst += src · k`
    fn add(dst: &mut Self::Px, src: &Self::Px, k: f32);
    fn pool<'a>(w: &'a mut Walker<'_>) -> &'a mut Vec<Box<[Self::Px; TILE_PX]>>;
}

struct Color;
struct Alpha;

impl Mode for Color {
    type Px = [f32; 4];
    const CLEAR: [f32; 4] = [0.0; 4];

    #[inline]
    fn alpha(p: &[f32; 4]) -> f32 {
        p[3]
    }

    #[inline]
    fn shade(path: &PreparedPath, px: f32, py: f32, cands: &[u32]) -> Option<[f32; 4]> {
        shade(path, px, py, cands)
    }

    #[inline]
    fn add(dst: &mut [f32; 4], src: &[f32; 4], k: f32) {
        for c in 0..4 {
            dst[c] += src[c] * k;
        }
    }

    fn pool<'a>(w: &'a mut Walker<'_>) -> &'a mut Vec<Box<Colors>> {
        &mut w.colors
    }
}

impl Mode for Alpha {
    type Px = f32;
    const CLEAR: f32 = 0.0;

    #[inline]
    fn alpha(p: &f32) -> f32 {
        *p
    }

    #[inline]
    fn shade(path: &PreparedPath, px: f32, py: f32, cands: &[u32]) -> Option<f32> {
        shade_alpha(path, px, py, cands)
    }

    #[inline]
    fn add(dst: &mut f32, src: &f32, k: f32) {
        *dst += src * k;
    }

    fn pool<'a>(w: &'a mut Walker<'_>) -> &'a mut Vec<Box<Alphas>> {
        &mut w.alphas
    }
}

fn narrow_opaque<M: Mode>(l: &mut Live, dst: &[M::Px; TILE_PX]) {
    retain(l, |i| M::alpha(&dst[i]) < OPAQUE);
}

/// Composites `layer`, scaled by `m`, behind `dst` on the pixels in `live`.
/// Returns how many of them became opaque.
fn composite<M: Mode>(
    layer: &[M::Px; TILE_PX],
    dst: &mut [M::Px; TILE_PX],
    live: &Live,
    m: &Alphas,
) -> u32 {
    let mut n = 0;
    for_each(live, |i| {
        let s = &layer[i];
        if M::alpha(s) <= 0.0 {
            return;
        }
        let d = &mut dst[i];
        let k = (1.0 - M::alpha(d)) * m[i];
        M::add(d, s, k);
        if M::alpha(d) >= OPAQUE {
            n += 1;
        }
    });
    n
}

struct TileInfo {
    x0: usize,
    y0: usize,
    w: usize,
    h: usize,
    /// The box of the pixel centres.
    rect: Aabb,
    /// The pixels inside the image.
    inside: Live,
}

impl TileInfo {
    fn new(i: usize, tiles_x: usize, width: usize, height: usize) -> TileInfo {
        let (x0, y0) = ((i % tiles_x) * TILE, (i / tiles_x) * TILE);
        let (w, h) = ((x0 + TILE).min(width) - x0, (y0 + TILE).min(height) - y0);
        let mut inside = [0; TILE];
        let row = if w == TILE { u32::MAX } else { (1 << w) - 1 };
        inside[..h].fill(row);
        TileInfo {
            x0,
            y0,
            w,
            h,
            rect: centres(x0, y0, w, h),
            inside,
        }
    }
}

/// The mask value for an element on a tile.
enum MaskOut {
    /// Nothing shows.
    Skip,
    /// Draw unmasked, on these pixels.
    One(Live),
    /// Draw scaled by these values, on these pixels.
    Scaled(Box<Alphas>, Live),
}

/// Per-thread state for rendering tiles.
struct Walker<'a> {
    prep: &'a Prepared<'a>,
    tile: TileInfo,
    cands: Vec<u32>,
    block_cands: Vec<u32>,
    /// Spare layers, one per level of nesting in use.
    colors: Vec<Box<Colors>>,
    alphas: Vec<Box<Alphas>>,
    /// Alpha of shared masks on the current tile.
    cache: Vec<(MaskId, Box<Alphas>)>,
}

impl<'a> Walker<'a> {
    fn new(prep: &'a Prepared<'a>) -> Walker<'a> {
        Walker {
            prep,
            tile: TileInfo::new(0, 1, 0, 0),
            cands: Vec::new(),
            block_cands: Vec::new(),
            colors: Vec::new(),
            alphas: Vec::new(),
            cache: Vec::new(),
        }
    }

    fn start(&mut self, tile: TileInfo) {
        self.tile = tile;
        let cache = std::mem::take(&mut self.cache);
        self.alphas.extend(cache.into_iter().map(|(_, b)| b));
    }

    fn take<M: Mode>(&mut self) -> Box<[M::Px; TILE_PX]> {
        M::pool(self)
            .pop()
            .unwrap_or_else(|| Box::new([M::CLEAR; TILE_PX]))
    }

    /// Draws `nodes`, front first, behind `dst` on the pixels in `live`.
    /// Returns how many of those pixels became opaque.
    fn draw_list<M: Mode>(
        &mut self,
        nodes: &[Node],
        dst: &mut [M::Px; TILE_PX],
        live: &Live,
    ) -> u32 {
        let mut live = *live;
        narrow_opaque::<M>(&mut live, dst);
        let mut remaining: u32 = live.iter().map(|r| r.count_ones()).sum();
        let mut done = 0;
        for n in nodes {
            if remaining == 0 {
                break;
            }
            let k = self.draw_node::<M>(n, dst, &live, None);
            debug_assert!(k <= remaining);
            remaining -= k;
            done += k;
        }
        done
    }

    /// Draws one node, scaled by `scale` if given.
    fn draw_node<M: Mode>(
        &mut self,
        node: &Node,
        dst: &mut [M::Px; TILE_PX],
        live: &Live,
        scale: Option<&Alphas>,
    ) -> u32 {
        let prep = self.prep;
        if !prep.node_reach(node).overlaps(&self.tile.rect) {
            return 0;
        }
        match node {
            Node::Path(p) => {
                match self.mask_value::<M>(scale, prep.paths[*p as usize].mask, dst, live) {
                    MaskOut::Skip => 0,
                    MaskOut::One(l) => self.paint::<M>(*p, dst, &l, None),
                    MaskOut::Scaled(m, l) => {
                        let n = self.paint::<M>(*p, dst, &l, Some(&m));
                        self.alphas.push(m);
                        n
                    }
                }
            }
            Node::Group(g) => {
                let pg = &prep.groups[*g as usize];
                self.draw_group::<M>(&pg.items, pg.mask, dst, live, scale)
            }
            Node::Masked { node, mask } => {
                match self.mask_value::<M>(scale, Some(*mask), dst, live) {
                    MaskOut::Skip => 0,
                    MaskOut::One(l) => self.draw_node::<M>(node, dst, &l, None),
                    MaskOut::Scaled(m, l) => {
                        let n = self.draw_node::<M>(node, dst, &l, Some(&m));
                        self.alphas.push(m);
                        n
                    }
                }
            }
        }
    }

    /// Draws a group: straight into `dst` where the mask is 1, or else
    /// through a layer of its own.
    fn draw_group<M: Mode>(
        &mut self,
        items: &[Node],
        mask: Option<MaskId>,
        dst: &mut [M::Px; TILE_PX],
        live: &Live,
        scale: Option<&Alphas>,
    ) -> u32 {
        match self.mask_value::<M>(scale, mask, dst, live) {
            MaskOut::Skip => 0,
            MaskOut::One(l) => self.draw_list::<M>(items, dst, &l),
            MaskOut::Scaled(m, l) => {
                let mut layer = self.take::<M>();
                layer.fill(M::CLEAR);
                self.draw_list::<M>(items, &mut layer, &l);
                let n = composite::<M>(&layer, dst, &l, &m);
                M::pool(self).push(layer);
                self.alphas.push(m);
                n
            }
        }
    }

    /// Combines an outer `scale` with an element's own `mask`, and narrows
    /// `live` to the pixels where the element can still show.
    fn mask_value<M: Mode>(
        &mut self,
        scale: Option<&Alphas>,
        mask: Option<MaskId>,
        dst: &[M::Px; TILE_PX],
        live: &Live,
    ) -> MaskOut {
        if scale.is_none() && mask.is_none() {
            return MaskOut::One(*live);
        }
        let mut l = *live;
        narrow_opaque::<M>(&mut l, dst);
        if is_empty(&l) {
            return MaskOut::Skip;
        }
        let mut m = self.take::<Alpha>();
        let Some(id) = mask else {
            // The caller already removed the pixels where the scale is 0.
            m.copy_from_slice(scale.unwrap());
            return MaskOut::Scaled(m, l);
        };
        self.mask_alpha(id, &l, &mut m);
        let invert = self.prep.masks[id as usize].invert;
        let mut all_one = true;
        retain(&mut l, |i| {
            // Layers stop filling at OPAQUE; count that as fully opaque.
            let a = if m[i] >= OPAQUE { 1.0 } else { m[i] };
            let mut v = if invert { 1.0 - a } else { a };
            if let Some(s) = scale {
                v *= s[i];
            }
            m[i] = v;
            if v > 0.0 && v < 1.0 {
                all_one = false;
            }
            v > 0.0
        });
        if is_empty(&l) {
            self.alphas.push(m);
            MaskOut::Skip
        } else if all_one {
            self.alphas.push(m);
            MaskOut::One(l)
        } else {
            MaskOut::Scaled(m, l)
        }
    }

    /// Writes the alpha of mask `id` into `out`, at least on the pixels in `live`.
    fn mask_alpha(&mut self, id: MaskId, live: &Live, out: &mut Alphas) {
        if !self.prep.masks[id as usize].shared {
            return self.render_mask(id, live, out);
        }
        if let Some((_, c)) = self.cache.iter().find(|(k, _)| *k == id) {
            out.copy_from_slice(&c[..]);
            return;
        }
        // Rendered on the whole tile, so it is valid for every later use.
        let inside = self.tile.inside;
        self.render_mask(id, &inside, out);
        let mut c = self.take::<Alpha>();
        c.copy_from_slice(&out[..]);
        self.cache.push((id, c));
    }

    fn render_mask(&mut self, id: MaskId, live: &Live, out: &mut Alphas) {
        out.fill(0.0);
        let pm = &self.prep.masks[id as usize];
        if !pm.reach.overlaps(&self.tile.rect) {
            return;
        }
        #[cfg(test)]
        self.prep.stats.mask_renders.fetch_add(1, Relaxed);
        self.draw_group::<Alpha>(&pm.items, pm.mask, out, live, None);
    }

    /// Paints path `p` behind `dst` on the pixels in `live`, scaled by
    /// `scale` if given. Returns how many pixels became opaque.
    fn paint<M: Mode>(
        &mut self,
        p: PathId,
        dst: &mut [M::Px; TILE_PX],
        live: &Live,
        scale: Option<&Alphas>,
    ) -> u32 {
        let prep = self.prep;
        let Some(path) = prep.geom(p) else { return 0 };
        let Walker {
            tile: t,
            cands,
            block_cands,
            ..
        } = self;
        if let Some(e) = path.extent
            && !path.poly.bbox.grow(e).overlaps(&t.rect)
        {
            return 0;
        }
        let lower = path.poly.initial_candidates(&t.rect, cands);
        if path.extent.is_some_and(|e| lower > e) {
            return 0;
        }
        let mut n = 0;
        for by in (0..t.h).step_by(BLOCK) {
            for bx in (0..t.w).step_by(BLOCK) {
                let (bw, bh) = (BLOCK.min(t.w - bx), BLOCK.min(t.h - by));
                let bits = (((1u64 << bw) - 1) << bx) as u32;
                if live[by..by + bh].iter().all(|r| r & bits == 0) {
                    continue;
                }
                let r = centres(t.x0 + bx, t.y0 + by, bw, bh);
                let lower = path.poly.filter_candidates(&r, cands, block_cands);
                if path.extent.is_some_and(|e| lower > e) {
                    continue;
                }
                for (y, &row) in live.iter().enumerate().take(by + bh).skip(by) {
                    let py = (t.y0 + y) as f32 + 0.5;
                    for x in bx..bx + bw {
                        if row >> x & 1 == 0 {
                            continue;
                        }
                        let i = y * TILE + x;
                        let d = &mut dst[i];
                        if M::alpha(d) >= OPAQUE {
                            continue;
                        }
                        let px = (t.x0 + x) as f32 + 0.5;
                        #[cfg(test)]
                        prep.stats.shaded.fetch_add(1, Relaxed);
                        let Some(src) = M::shade(path, px, py, block_cands) else {
                            continue;
                        };
                        let mut k = 1.0 - M::alpha(d);
                        if let Some(s) = scale {
                            k *= s[i];
                        }
                        M::add(d, &src, k);
                        if M::alpha(d) >= OPAQUE {
                            n += 1;
                        }
                    }
                }
            }
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse;

    fn scene(src: &str) -> Scene {
        let p = parse(src).unwrap();
        p.scene.unwrap_or_else(|| panic!("{:?}", p.diagnostics))
    }

    struct Out {
        c: Canvas,
        prepared: u64,
        shaded: u64,
        mask_renders: u64,
    }

    fn run(src: &str) -> Out {
        let s = scene(src);
        let p = Prepared::new(&s);
        let c = render_prepared(&p);
        let st = &p.stats;
        Out {
            c,
            prepared: st.prepared.load(Relaxed),
            shaded: st.shaded.load(Relaxed),
            mask_renders: st.mask_renders.load(Relaxed),
        }
    }

    fn pixels(c: &Canvas) -> Vec<[u8; 4]> {
        (0..c.height)
            .flat_map(|y| (0..c.width).map(move |x| c.pixel(x, y)))
            .collect()
    }

    fn close(a: &Canvas, b: &Canvas) -> bool {
        pixels(a)
            .iter()
            .zip(pixels(b))
            .all(|(p, q)| p.iter().zip(q).all(|(x, y)| x.abs_diff(y) <= 1))
    }

    /// A path covering the whole image in one colour.
    fn fill(l: f32, a: f32) -> String {
        format!("path {{ point 0 0; point 1 0; gradient 0 {{ stop 0 {l} 0 0 {a}; }} }}")
    }

    /// A hard-edged horizontal band of half-width `r` around `y`.
    fn hband(y: f32, r: f32, a: f32) -> String {
        format!(
            "path {{ point -1000 {y}; point 1000 {y}; gradient 0 {{ stop 0 0 0 0 {a}; stop {r} 0 0 0 {a}; stop {r} 0 0 0 0; }} }}"
        )
    }

    /// A hard-edged vertical band of half-width `r` around `x`.
    fn vband(x: f32, r: f32, l: f32, a: f32) -> String {
        format!(
            "path {{ point {x} -1000; point {x} 1000; gradient 0 {{ stop 0 {l} 0 0 {a}; stop {r} {l} 0 0 {a}; stop {r} {l} 0 0 0; }} }}"
        )
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
        let c = render(&s);
        assert_eq!(c.pixel(35, 20), [255, 0, 0, 255]);
        assert_eq!(c.pixel(35, 17), [255, 0, 0, 255]);
        assert_eq!(c.pixel(35, 16), [0, 0, 255, 255]);
        assert_eq!(c.pixel(0, 39), [0, 0, 255, 255]);
        assert!(c.is_opaque());
    }

    #[test]
    fn semi_transparent_front_composites_over_back() {
        let c = render(&scene(&format!(
            "image width=4 height=4\n{}\n{}",
            fill(1.0, 0.5),
            fill(0.0, 1.0)
        )));
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
        let c = render(&s);
        assert_eq!(c.pixel(0, 0)[3], 0);
        assert_eq!(c.pixel(25, 25)[3], 0);
        // The path runs clockwise on screen, so its left side is outside.
        assert!(c.pixel(25, 9)[3] > 0);
        assert_eq!(c.pixel(25, 11)[3], 0);
    }

    #[test]
    fn unrenderable_paths_paint_nothing() {
        let o = run(
            "image width=4 height=4\npath { point 1 1; point 1 1; gradient 0 { stop 0 0 0 0; } }\npath { point 0 0; point 1 1 }",
        );
        assert!(pixels(&o.c).iter().all(|p| *p == [0; 4]));
        assert_eq!(o.prepared, 0);
    }

    #[test]
    fn unmasked_group_is_its_children() {
        let (a, b, c) = (
            vband(10.0, 4.0, 0.9, 0.6),
            hband(20.0, 6.0, 0.7),
            fill(0.3, 1.0),
        );
        let flat = run(&format!(
            "image width=40 height=40\n{a}\n{b}\n{a}\n{b}\n{c}"
        ));
        let nested = run(&format!(
            "image width=40 height=40\ngroup {{ {a}; group {{ {b} }} }}\ngroup {{ group {{ }}; {a} }}\n{b}\n{c}"
        ));
        assert_eq!(flat.c.px, nested.c.px);
        // A reused group is drawn straight into its parent too.
        let reused = run(&format!(
            "image width=40 height=40\ngroup id=\"g\" {{ {a}; group {{ {b} }} }}\nuse \"g\"\n{c}"
        ));
        assert_eq!(flat.c.px, reused.c.px);
    }

    #[test]
    fn masked_group_uses_a_layer() {
        let mask = format!("mask {{ {} }}", fill(0.0, 0.5));
        let (w, k) = (fill(1.0, 0.5), fill(0.0, 0.5));
        let grouped = render(&scene(&format!(
            "image width=4 height=4\ngroup {{ {mask}; {w}; {k} }}"
        )));
        // The layer is white at .5 over black at .5: premultiplied (.5, .5, .5, .75),
        // then scaled by .5.
        let expect = to_rgba8([0.25, 0.25, 0.25, 0.375]);
        let p = grouped.pixel(2, 2);
        assert!(
            p.iter().zip(expect).all(|(a, b)| a.abs_diff(b) <= 1),
            "{p:?} vs {expect:?}"
        );
        let separate = render(&scene(&format!(
            "image width=4 height=4\npath {{ {mask}; point 0 0; point 1 0; gradient 0 {{ stop 0 1 0 0 0.5; }} }}\n\
             path {{ {mask}; point 0 0; point 1 0; gradient 0 {{ stop 0 0 0 0 0.5; }} }}"
        )));
        // (.25, .25, .25, .25) with black at .5 · .5 behind: alpha .4375.
        let expect = to_rgba8([0.25, 0.25, 0.25, 0.4375]);
        let q = separate.pixel(2, 2);
        assert!(
            q.iter().zip(expect).all(|(a, b)| a.abs_diff(b) <= 1),
            "{q:?} vs {expect:?}"
        );
        assert_ne!(p, q);
    }

    #[test]
    fn inverted_and_empty_masks() {
        let band = hband(8.0, 8.0, 1.0); // rows 0..16
        let el = |mask: &str| {
            format!(
                "image width=16 height=32\ngroup {{ {mask}; {} }}",
                fill(0.5, 1.0)
            )
        };
        let c = render(&scene(&el(&format!("mask {{ {band} }}"))));
        assert_eq!((c.pixel(3, 15)[3], c.pixel(3, 16)[3]), (255, 0));
        let c = render(&scene(&el(&format!("mask invert=#true {{ {band} }}"))));
        assert_eq!((c.pixel(3, 15)[3], c.pixel(3, 16)[3]), (0, 255));
        let o = run(&el("mask { }"));
        assert!(pixels(&o.c).iter().all(|p| p[3] == 0));
        assert_eq!(o.shaded, 0);
        let c = render(&scene(&el("mask invert=#true { }")));
        assert!(pixels(&c).iter().all(|p| p[3] == 255));
    }

    #[test]
    fn mask_union_and_intersection() {
        let (top, left) = (hband(4.0, 4.0, 1.0), vband(4.0, 4.0, 0.0, 1.0)); // rows 0..8, columns 0..8
        let el = |mask: &str| {
            format!(
                "image width=16 height=16\ngroup {{ {mask}; {} }}",
                fill(0.5, 1.0)
            )
        };
        let c = render(&scene(&el(&format!("mask {{ {top}; {left} }}"))));
        let vis = |c: &Canvas| {
            [
                c.pixel(2, 2)[3],
                c.pixel(12, 2)[3],
                c.pixel(2, 12)[3],
                c.pixel(12, 12)[3],
            ]
        };
        assert_eq!(vis(&c), [255, 255, 255, 0]);
        let c = render(&scene(&el(&format!("mask {{ mask {{ {top} }}; {left} }}"))));
        assert_eq!(vis(&c), [255, 0, 0, 0]);
        // Subtraction: an inverted mask inside a mask.
        let c = render(&scene(&el(&format!(
            "mask {{ mask invert=#true {{ {top} }}; {left} }}"
        ))));
        assert_eq!(vis(&c), [0, 0, 255, 0]);
    }

    #[test]
    fn mask_clips_reach() {
        // 4 × 4 tiles; the mask only covers the first rows of the top tiles.
        let src = format!(
            "image width=128 height=128\npath {{ mask {{ {} }}; point 0 0; point 1 0; gradient 0 {{ stop 0 0.5 0 0; }} }}",
            hband(2.0, 2.0, 1.0)
        );
        let o = run(&src);
        assert_eq!(o.c.pixel(50, 1)[3], 255);
        assert_eq!(o.c.pixel(50, 40)[3], 0);
        // Only the top row of tiles is touched, and only its first block row
        // in the mask (the culling skips the rest).
        assert_eq!(o.mask_renders, 4);
        assert!(o.shaded <= 4 * (8 * 32) * 2, "{}", o.shaded);
    }

    fn path_reach(src: &str, i: usize) -> Reach {
        let s = scene(src);
        Prepared::new(&s).paths[i].reach
    }

    #[test]
    fn filled_shapes_have_bounded_reach() {
        let img = "image width=128 height=128\n";
        // Clockwise on screen: the left side (t = 1) is outside.
        // Hobby's curve through these is nearly a circle, of radius 8.5 around (14, 14).
        let cw = "point 8 8; point 20 8; point 20 20; point 8 20";
        let ccw = "point 8 8; point 8 20; point 20 20; point 20 8";
        let outer = |t: u32| format!("gradient {t} {{ stop 0 0 0 0; stop 2 0 0 0 0; }}");
        let solid = |t: u32| format!("gradient {t} {{ stop 0 0 0 0; }}");
        let shape = |pts: &str, a: u32, b: u32| {
            format!(
                "{img}path closed=#true {{ {pts}; {}; {} }}",
                outer(a),
                solid(b)
            )
        };
        let Reach::Box(b) = path_reach(&shape(cw, 1, 3), 0) else {
            panic!("unbounded")
        };
        assert!(
            b.x0 > 2.0 && b.x1 < 26.0 && b.y0 > 2.0 && b.y1 < 26.0,
            "{b:?}"
        );
        assert!(matches!(path_reach(&shape(ccw, 3, 1), 0), Reach::Box(_)));
        // The solid side is outside: unbounded.
        assert!(matches!(path_reach(&shape(cw, 3, 1), 0), Reach::All));
        assert!(matches!(path_reach(&shape(ccw, 1, 3), 0), Reach::All));
        // A figure 8 can reach out from either side.
        let eight = "point 8 8; point 20 20; point 20 8; point 8 20";
        assert!(matches!(path_reach(&shape(eight, 1, 3), 0), Reach::All));

        // A full-image path under a filled-shape mask only touches one tile.
        let o = run(&format!(
            "{img}path {{ mask {{ path closed=#true {{ {cw}; {}; {} }} }}; point 0 0; point 1 0; gradient 0 {{ stop 0 0.5 0 0; }} }}",
            outer(1),
            solid(3)
        ));
        assert_eq!(o.mask_renders, 1);
        assert_eq!(o.c.pixel(14, 14)[3], 255);
        assert_eq!(o.c.pixel(14, 30)[3], 0);
        assert!(o.shaded <= 2 * 1024);
    }

    #[test]
    fn inverted_opaque_mask_is_exactly_zero() {
        // Two paths at .99 make the mask alpha .9999: past OPAQUE, not 1.
        let o = run(&format!(
            "image width=8 height=8\ngroup {{ mask invert=#true {{ {}; {} }}; {} }}",
            fill(0.0, 0.99),
            fill(0.0, 0.99),
            fill(0.5, 1.0)
        ));
        assert!(o.c.px.iter().all(|p| p[3] == 0.0));
        // Only the mask's two paths were shaded.
        assert_eq!(o.shaded, 2 * 64);
    }

    #[test]
    fn live_sets_limit_shading() {
        // Opaque columns 0..16 in front; behind, a group masked at .5 to rows 0..16.
        let front = vband(8.0, 8.0, 0.2, 1.0);
        let o = run(&format!(
            "image width=32 height=32\n{front}\ngroup {{ mask {{ {} }}; {}; {} }}",
            hband(8.0, 8.0, 0.5),
            fill(1.0, 0.5),
            fill(0.0, 0.5)
        ));
        // Front: its two 8-wide block columns, plus the next one, which block
        // culling cannot rule out. Mask: only where the front is not opaque,
        // in the 16 rows it covers plus 8 more that culling keeps. Children:
        // only where the front is not opaque and the mask is not 0.
        assert_eq!(o.shaded, (512 + 256) + (256 + 128) + 256 + 256);
        assert!(o.c.pixel(20, 20)[3] == 0 && o.c.pixel(20, 4)[3] > 0);
    }

    #[test]
    fn opaque_content_stops_the_walk() {
        let o = run(&format!(
            "image width=100 height=100\n{}\n{}",
            fill(0.5, 1.0),
            fill(0.2, 1.0)
        ));
        assert_eq!(o.prepared, 1);
        assert_eq!(o.shaded, 100 * 100);
    }

    #[test]
    fn use_repeats_the_definition() {
        let a = vband(10.0, 4.0, 0.9, 0.6).replacen("path {", "path id=\"a\" {", 1);
        let b = hband(20.0, 6.0, 0.6);
        let written = render(&scene(&format!(
            "image width=40 height=40\n{a}\n{b}\n{}\n{b}",
            vband(10.0, 4.0, 0.9, 0.6)
        )));
        let used = run(&format!(
            "image width=40 height=40\n{a}\ngroup id=\"b\" {{ {b} }}\nuse \"a\"\nuse \"b\""
        ));
        assert_eq!(written.px, used.c.px);
        assert_eq!(used.prepared, 2);

        // `use` with a mask matches a masked group around a copy.
        let mask = format!("mask {{ {} }}", hband(10.0, 10.0, 0.5));
        let with_use = render(&scene(&format!(
            "image width=40 height=40\ndefs {{ {a} }}\nuse \"a\" {{ {mask} }}\n{}",
            fill(0.1, 1.0)
        )));
        let with_group = render(&scene(&format!(
            "image width=40 height=40\ngroup {{ {mask}; {} }}\n{}",
            vband(10.0, 4.0, 0.9, 0.6),
            fill(0.1, 1.0)
        )));
        assert!(close(&with_use, &with_group));
        // On a group definition with a mask of its own, the masks multiply.
        let g = format!(
            "group id=\"g\" {{ mask {{ {} }}; {b} }}",
            vband(20.0, 10.0, 0.0, 0.5)
        );
        let with_use = render(&scene(&format!(
            "image width=40 height=40\ndefs {{ {g} }}\nuse \"g\" {{ {mask} }}"
        )));
        let with_group = render(&scene(&format!(
            "image width=40 height=40\ngroup {{ {mask}; {} }}",
            g.replace(" id=\"g\"", "")
        )));
        assert!(close(&with_use, &with_group));
    }

    #[test]
    fn a_path_used_many_times_is_prepared_once() {
        let a = vband(10.0, 4.0, 0.9, 0.1).replacen("path {", "path id=\"a\" {", 1);
        let o = run(&format!(
            "image width=64 height=64\ndefs {{ {a} }}\nuse \"a\"\nuse \"a\"\ngroup {{ mask {{ use \"a\" }}; use \"a\" }}\nuse \"a\" {{ mask invert=#true {{ use \"a\" }} }}"
        ));
        assert_eq!(o.prepared, 1);
    }

    #[test]
    fn shared_masks_are_cached() {
        let spot = "path id=\"spot\" closed=#true { point 4 4; point 28 4; point 16 28; gradient 1 { stop 0 0 0 0 0.7; stop 6 0 0 0 0; }; gradient 3 { stop 0 0 0 0 0.7; } }";
        let (x, y) = (vband(10.0, 6.0, 0.9, 0.6), hband(12.0, 5.0, 0.6));
        let shared = run(&format!(
            "image width=32 height=32\ndefs {{ {spot} }}\ngroup {{ mask {{ use \"spot\" }}; {x} }}\ngroup {{ mask {{ use \"spot\" }}; {y} }}"
        ));
        let unshared_mask = spot.replace(" id=\"spot\"", "");
        let separate = run(&format!(
            "image width=32 height=32\ngroup {{ mask {{ {unshared_mask} }}; {x} }}\ngroup {{ mask {{ {unshared_mask} }}; {y} }}"
        ));
        assert_eq!(shared.mask_renders, 1);
        assert_eq!(separate.mask_renders, 2);
        assert_eq!(pixels(&shared.c), pixels(&separate.c));
        assert!(pixels(&shared.c).iter().any(|p| p[3] > 0));
    }

    #[test]
    fn deep_nesting_renders() {
        let mut s = String::from("image width=40 height=40\n");
        let depth = 20;
        for i in 0..depth {
            s += &format!(
                "group {{ mask {{ {} }}\n",
                hband(20.0, 20.0 - i as f32 * 0.5, 0.95)
            );
        }
        s += &fill(0.6, 1.0);
        s += &" }".repeat(depth);
        let c = render(&scene(&s));
        assert!(c.pixel(20, 20)[3] > 0 && c.pixel(20, 1)[3] == 0);
    }
}
