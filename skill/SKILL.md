---
name: szeel
description: Renders PNG images with szeel, a command-line renderer that colours a picture by soft OKLCH gradients flowing outward from smooth curves (Hobby paths). Use when the user asks for an image, picture, illustration, wallpaper, background, glow, abstract or generative art, or mentions szeel or a .kdl scene.
---

# szeel

szeel turns a text scene (`.kdl`) into a PNG. Every pixel takes its colour
from its **nearest point on a smooth curve**, using colour ramps that run
outward from the curve. The results are soft glows, luminous strokes, smooth
colour fields and auras. It cannot draw text, photos, hard-edged flat shapes
or detailed figures. Interpret requests in that style, and say so if the
request needs something it cannot do.

## Setup (once per conversation)

The binary is `bin/szeel` next to this file. It is a static x86-64 Linux build,
so it needs no libraries. The skill folder may be read-only or lose the
executable bit, so copy it first:

```bash
SKILL_DIR=$(dirname "$(find / -path '*/szeel/SKILL.md' 2>/dev/null | head -1)")
cp "$SKILL_DIR/bin/szeel" /tmp/szeel && chmod +x /tmp/szeel
/tmp/szeel scene.kdl out.png
```

Errors and warnings are printed with the file position and a hint. A warning
still produces an image, so read the warnings anyway (for example "this path
has no usable gradients and paints nothing").

If `bin/szeel` is missing, say so plainly and deliver the `.kdl` unrendered.
szeel is not on npm, PyPI or crates.io, and you must never imitate its output
with another renderer.

## Workflow

1. Write the scene to a `.kdl` file. Hand-write simple scenes. For anything
   with more than ~15 paths, write a **Python generator** that emits the KDL
   (helpers for dots, soft shapes and masks, loops for textures), and edit the
   generator rather than the KDL.
2. Render it. While you iterate, use a small size (for example 800×450) to keep
   it fast, then scale every coordinate and distance up for the final render.
   Even 2,000+ paths render in a few seconds.
3. **Look at the PNG** (open the image file) and compare it with what was asked
   for. Also **crop and enlarge problem areas** with PIL, because small
   artifacts (loops, seams, edge bands) are invisible at full size. When a
   technique is uncertain, test it alone in a small scene first. Expect several
   rounds.
4. Give the user the final PNG as a downloadable file. If `/mnt/user-data/outputs/`
   exists, save it there. Also give them the `.kdl` and the generator script,
   so they can ask for changes later.

## Scene format

A [KDL v2](https://kdl.dev) document. Booleans are `#true` / `#false`. Several
nodes can go on one line separated by `;`. Comments are `//`.

```kdl
image width=1600 height=900   // must come first
path { … }                    // drawn in FRONT of everything below it
path { … }                    // further back
```

**Order is front first.** Each element is composited _behind_ everything before
it. Put the background **last**. Pixels that nothing covers are transparent.

### Paths

```kdl
path closed=#false id="name" {     // both properties optional
    point 100 500                  // x y in pixels, y points DOWN
    point 400 150
    point 700 500
    gradient 1.5 { stop 0 0.75 0.18 40; stop 60 0.75 0.18 40 0; }
    gradient 3.5 { stop 0 0.55 0.12 250; stop 80 0.55 0.12 250 0; }
    // optionally one `mask { … }`
}
```

- The points are joined by a smooth curve through all of them (Hobby's
  algorithm, like MetaPost). Two points give a straight segment. A closed path
  loops back smoothly to the first point.
- Two almost identical points (`point 400 300; point 400.01 300`) act as a
  **dot**: its gradients become a radial glow.

### Gradients: `gradient t { stop … }`

`t` says **where around the path** the gradient sits. Each pixel blends the
two gradients on either side of its own `t`, wrapping around, so a single
gradient applies all the way round.

Open path, `t` in `[0, 4)`, going once around the curve:

| `t`   | where                                                                |
| ----- | -------------------------------------------------------------------- |
| 0 → 1 | the cap around the **start** point (right side → behind → left side) |
| 1 → 2 | the **left** side, from start (1) to end (2)                         |
| 2 → 3 | the cap around the **end** point (left side → ahead → right side)    |
| 3 → 4 | the **right** side, from end (3) back to start (4)                   |

"Left" and "right" are as seen on screen when you travel from the first point
to the last. Example: a path drawn left to right has its **left side above**
it and its right side below it. `t = 1.5` is above the middle, and `t = 3.5`
is below the middle.

Closed path: only the two sides exist. Left is `[1, 2)` and right is `[3, 4)`.
Gradients with other `t` values are ignored, with a warning. If the points go
**clockwise on screen**, the **inside is the right side** (`3…4`) and the
outside is the left (`1…2`). If they go anticlockwise, it's the other way round.

To test orientation in code (y down): the shoelace sum
`Σ(x_i·y_{i+1} − x_{i+1}·y_i)` is **positive for clockwise on screen**. Reverse
the points if it is negative.

**`t` per point on a closed path.** With N points in clockwise order, point k
sits at `t = 4 − k/N` on the inside (point 0 at `t = 3.0`). The right side
runs _backwards_ through the points. Outside, point k sits at `t = 1 + k/N`.
One gradient per point therefore gives each part of the outline its own
colour. This is how you light a shape from one direction: lighter gradients
on the points facing the light, darker ones opposite. Write `t` with 4
decimals and keep it below `4` (for example `3.9987`). Rounding to `4` is out
of range and triggers a warning.

### Stops: `stop distance L C H [alpha]`

- `distance`: pixels away from the curve, 0 or more. Distances must not
  decrease within a gradient.
- `L C H`: an **OKLCH** colour. L is lightness from 0 to 1. C is chroma, from 0
  (grey) up to about 0.3 (most vivid; 0.1–0.2 is typical). H is hue in degrees:
  roughly 30 red, 70 orange, 100 yellow, 145 green, 200 cyan, 250 blue,
  300 purple, 350 pink.
- `alpha`: from 0 to 1, default 1.
- **Hue is interpolated numerically**, not the short way round the colour
  wheel. Blending H 355 → 8 passes through cyan (a green band appears in the
  middle). Write the neighbour as `-5` or `368` instead, and for warm ↔ cool
  blends (245 → 80) use `440` so the blend passes through violet, not green.
  Values outside 0–360 are accepted.
- Beyond the last stop, its colour continues **forever**. So:
  - End with `alpha 0` for a stroke or glow of limited width:
    `stop 0 0.8 0.15 40; stop 50 0.8 0.15 40 0`.
  - End opaque to fill everything out to the image edges. This is how
    backgrounds and colour fields work.

### Groups, masks and reuse

```kdl
group id="g" { path {…}; path {…}; mask {…} }    // a group matters only with a mask
mask invert=#false { path {…}; group {…}; use "id" }
defs { path id="shape" {…} }                     // defined but not drawn
use "shape" { mask {…} }                         // draw an earlier id again
```

- A `mask` goes inside a `path`, `group`, `use` or another `mask`, at most one
  per element. Only the mask's **alpha** counts; its colours are ignored
  (write `0 0 0`). The element is multiplied by the mask's alpha, or by
  1 − alpha with `invert=#true`.
- Several shapes in one mask make a union. A mask inside a mask makes an
  intersection. An inverted mask inside a mask subtracts.
- An `id` must be unique and defined before any `use` of it. Everything inside
  `defs` is drawn only where it is used.
- Limits: at most 1,000,000 paths drawn in total, counting every `use`, and at
  most 64 levels of nesting.
- Nodes on the same line need `;` between them (`} path {` is a syntax error).
  When generating, join sibling nodes with newlines.
- A mask shape must be **solid with an antialiased edge**:
  `gradient 3.5 { stop 0 0 0 0 0.5; stop 1 0 0 0 1; }` inside and
  `gradient 1.5 { stop 0 0 0 0 0.5; stop 1 0 0 0 0; }` outside (clockwise points).
- **Mask each object once.** Put all of an object's layers in one `group` with
  one `mask { use "shape" }`, and end the group with an opaque full-canvas base
  fill. If every layer carries its own copy of the mask, the half-transparent
  edges stack up and draw a dark outline round the object.

## Recipes

Background filling the whole image, put **last**. A single opaque gradient
covers everything:

```kdl
path { point 0 0; point 1 0; gradient 0 { stop 0 0.15 0.04 270; } }
```

Background that changes smoothly from top to bottom: a horizontal line
through the middle, with different colours above (left) and below (right).
Both sides **start with the same colour at distance 0**. If they started with
different colours, there would be a hard edge along the line, which is useful
for a horizon.

```kdl
path {
    point 0 450; point 1600 450
    gradient 1.5 { stop 0 0.5 0.13 340; stop 450 0.12 0.05 270; }
    gradient 3.5 { stop 0 0.5 0.13 340; stop 450 0.2 0.08 20; }
}
```

Glowing stroke whose colour changes along its length (start → end on the left
side):

```kdl
path {
    point 200 600; point 600 300; point 1000 500; point 1400 250
    gradient 1.0 { stop 0 0.85 0.2 30;  stop 70 0.6 0.2 30 0; }
    gradient 2.0 { stop 0 0.85 0.2 300; stop 70 0.6 0.2 300 0; }
    gradient 3.0 { stop 0 0.85 0.2 300; stop 70 0.6 0.2 300 0; }
}
```

To make a **round, even end cap**, put matching gradients just either side of
the cap, for example at `3.9` and `1.1` for the start, or `1.9` and `3.1` for
the end.

Soft blob: a closed path drawn clockwise, light inside and fading out
outside:

```kdl
path closed=#true {
    point 600 300; point 900 350; point 850 600; point 550 550
    gradient 3.5 { stop 0 0.9 0.1 80; stop 120 0.98 0.02 80; }
    gradient 1.5 { stop 0 0.9 0.1 80; stop 60 0.9 0.1 80 0; }
}
```

Hole punched through an element:

```kdl
path {
    mask invert=#true {
        path { point 780 450; point 820 450; gradient 0 { stop 0 0 0 0; stop 40 0 0 0 0; } }
    }
    point 0 0; point 1 0
    gradient 0 { stop 0 0.95 0.02 90; }
}
```

## Pitfalls

- **Hobby loops at sharp turns.** A closed sliver with pointed tips loops and
  bulges at the tips, and pinches in the middle. Avoid turns near 180°: sample
  outlines densely (8–12 points per side), leave out the single tip point so
  the ends are blunt, and never build a shape from two halves that meet at a
  sharp point (they leave a gap). For thin marks use the _tapered field_
  recipe below instead.
- **Wedge-shaped shards.** An inside gradient's last stop carries on forever,
  so if a far-away pixel is assigned to the inside near a tip, it shows a
  wedge. For thin shapes, end the inside gradient with `alpha 0` just past the
  half-width: `stop 0 … 0; stop e … a; stop w+0.5 … a; stop w+1.5 … 0`.
- **Seams inside a closed shape.** Where a pixel is equally far from two parts
  of the outline with different gradients, a hard line appears (the medial
  axis). Vary per-point gradients only near the edge (≤ ~70 px) and make every
  gradient reach the **same colour** at larger distances. Direct the lighting
  with separate masked glows instead.
- **Bands at the image border.** Shapes that end at the canvas edge fade there.
  Extend them 40–80 px beyond the canvas.
- **A wide dark halo is not a shadow.** Keep contact darkening tight, or
  objects float on a muddy patch. Ground objects with a crisp dark contact
  line plus a softer cast shadow.

## Building detailed pictures

**Outline, then masks.** Do not draw solid objects as thick strokes; it looks
primitive. Define each object's **outline** once in `defs` as a solid mask
shape. Then paint its interior with fields clipped by that mask:

- glows: dots placed at highlights, core shadows or reflected light
- shading across a narrow form: an _open_ path along its centre line, extended
  past both ends, with different gradients on its left and right sides,
  masked by the outline
- reflections and textures: their own shapes, masked by the outline (a mask
  nested inside the mask can fade them out)

**Tapered field (thin strokes, streaks).** Use an open path with fading caps
and narrow ends:

```kdl
path { point 400 300; point 420 420; point 410 560
  gradient 0.5 { stop 0 0.7 0.17 55 0; stop 1 0.7 0.17 55 0; }   // caps invisible
  gradient 2.5 { stop 0 0.7 0.17 55 0; stop 1 0.7 0.17 55 0; }
  gradient 1.05 { stop 0 0.7 0.17 55 0.05; stop 2 0.7 0.17 55 0; }   // thin ends
  gradient 1.5  { stop 0 0.7 0.17 55 0.2; stop 6 0.7 0.17 55 0; }    // wide middle
  gradient 1.95 { stop 0 0.7 0.17 55 0.05; stop 2 0.7 0.17 55 0; }
  gradient 3.05 { stop 0 0.7 0.17 55 0.05; stop 2 0.7 0.17 55 0; }
  gradient 3.5  { stop 0 0.7 0.17 55 0.2; stop 6 0.7 0.17 55 0; }
  gradient 3.95 { stop 0 0.7 0.17 55 0.05; stop 2 0.7 0.17 55 0; } }
```

**Accurate form and light: contours of computed fields.** When shading must
be convincing, compute it in Python instead of placing it by eye: model the
scene simply (for example signed-distance functions ray-marched with numpy at
a low resolution) and compute per-pixel fields such as lighting, shadows,
highlights and texture. Then turn each field into szeel layers:

1. Fill pixels outside the object with the nearest valid value and pad the
   array by ~40 px, so contours run past silhouettes and image edges.
2. For each level, take the filled iso-regions (for example with `contourpy`),
   resample to ~8–10 px spacing, smooth lightly and orient clockwise.
3. Emit one closed path per region with one gradient per point: alpha 0 at
   the edge, rising to the layer's alpha over a **feather equal to that
   point's distance to the next level's region** (capped). Stacked levels then
   blend smoothly. Subtract holes with `mask invert=#true`.
4. Place small surface details using the model's projected geometry. Marks
   placed by eye in 2D look as if they float off the surface.

## Art direction

A technically right render is not the goal. Aim for a handcrafted painting.

- **Character over idealisation:** irregular, natural shapes with small
  imperfections. Avoid perfectly symmetric, icon-like shapes.
- **Every element earns its place:** each part is clearly explained, or it is
  left out.
- **One focal area:** put the most contrast, the most saturated colour, the
  finest texture and the strongest light there. Let the rest go quiet and
  plain. Fade textures out in shadow and away from the focus. Let a shadow
  edge dissolve into a background of similar value (a lost edge).
- **Colour:** saturation peaks where light turns to shadow. Shadows are
  coloured (often cooler), with warm light bouncing back from nearby
  surfaces. A complementary accent makes the main colour sing.
- **Depth:** a crisp, plain foreground edge, a detailed middle, and a
  distance that fades into haze or darkness.
- **Key:** decide early between low-key (dark, dramatic) and high-key
  (bright). In high key, keep only a few small darks to hold the focus. If the
  user hasn't said, a bright picture is the safer default.

## Tips

- Sketch the layout first: decide where the curves go and which side faces
  where, then choose colours.
- Larger distances (200–600 px) give soft, painterly fields. Small ones (5–40 px)
  give crisp lines.
- Where two curves or two arms of one curve are about equally close, there is
  a sharp seam. Such seams can look striking, but if you don't want one, move
  the curves apart or fade the front curve out with `alpha 0` sooner.
- Keep L and C of neighbouring stops similar for smooth, gentle blends. Vary H
  for rainbow effects.
- Tell the user the render size, and offer changes such as a different palette,
  a different size or more glow.
