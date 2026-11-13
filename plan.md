# Plan: Hobby-path image renderer

## Goal

A command-line program that reads a text file describing an image as a stack of
paths built with **Hobby's algorithm**, renders them, and writes the result as a
**PNG** image.

```
<program> <input.txt> <output.png>
```

Both the input description file and the output PNG path are given as command-line
parameters.

**Priorities:** the tool must be **very fast** and **memory efficient**. It runs on
the **CPU only** for now. The design should leave room for a GPU implementation
later.

**Language:** Rust.

## Rendering model

- An image consists of multiple paths, listed in the input file.
- Paths are rendered **in sequence from the top of the file to the bottom**.
- The **first path is frontmost**: each later path only fills pixels that are still
  **transparent or semi-transparent** (i.e. it is composited *behind* what has
  already been drawn — "destination-over" compositing).
- Rendering stops early when either:
  - every pixel of the image is fully opaque, or
  - all paths have been drawn.
- The final buffer is encoded as PNG (with alpha channel).

## Paths

- Each path is a curve through a list of points, with the smooth curve between
  them computed by Hobby's algorithm (cubic Bézier segments with control points
  chosen by Hobby's rules). For now, all points use the default tension (1) and
  the ends use the default curl (1). Per-point tension, curl and explicit
  directions are not supported.
- Paths may be **open** or **closed**.
- An open path has a **start point** and an **end point**. Every path has a
  **direction** of travel. "Left" and "right" sides are relative to that direction.
- **A pixel is painted by the closest point on the path.** The pixel's colour
  depends on where that closest point is (its `t` coordinate, see below) and on
  how far the pixel is from it.

## Path width

- A path has **no explicit width**.
- If **all** of a path's gradients end with a transparent colour, the path has an
  **implicit width**: the pixels beyond the transparent ends are left unpainted.
- Otherwise the path is expected to **paint the entire image**.

## Gradient coordinate `t`

Each path's colour is defined by gradients placed around the path using a
coordinate `t` that walks all the way around it, like going around the outline
of a stroke.

**Positions along the path are measured by arc length.** For example, `t = 1.5`
is the point halfway along the curve's length, not halfway through its Bézier
parameter.

### Open paths

| `t`           | Location                                                              |
| ------------- | --------------------------------------------------------------------- |
| `0`           | Start point, **right** side, perpendicular to the path direction      |
| `0 → 1`       | Rotating around the **start point** from the right side to the left   |
| `1`           | Start point, **left** side, perpendicular                             |
| `1 → 2`       | Along the **left** side, from start to end                            |
| `1.5`         | Midpoint of the path (by length), left side                           |
| `2`           | End point, **left** side, perpendicular                               |
| `2 → 3`       | Rotating around the **end point** from the left side to the right     |
| `3`           | End point, **right** side, perpendicular                              |
| `3 → 4`       | Along the **right** side, from end back to start                      |
| `3.5`         | Midpoint of the path (by length), right side                          |
| `4`           | Same as `t = 0` (the coordinate is cyclic)                            |

Around an endpoint, `t` is **linear in the angle**: one unit of `t` is a half-turn.

### Closed paths

- Only gradients in the ranges **`1 ≤ t < 2`** (left side) and **`3 ≤ t < 4`**
  (right side) are used.
- Gradients defined outside these ranges are **ignored**. A closed path has no
  endpoints, so it has no endpoint rotations.
- **Each side wraps on itself**: on the left side, `t` just below 2 joins back
  to `t = 1`; on the right side, `t` just below 4 joins back to `t = 3`.
  Gradient neighbours are found within the side's own loop.
- If a side has **no gradients**, that side is **left unpainted**.

## Gradients

- A path has one or more **gradients**, each placed at a `t` value.
- Each gradient is a colour ramp running **outward from the path**. Its stops are
  placed at distances from the path, given **in pixels**. The colour after its
  last stop is what decides whether the path has an implicit width (see "Path
  width").
- Along the path, a pixel's colour is **interpolated between the two gradients
  next to its `t`** (its neighbours).
- Neighbours are found **in an unbroken loop**: around the whole path for open
  paths (`t = 4 ≡ 0`), and around each side separately for closed paths. Past the
  last gradient, the next neighbour is the first one again.
- **A single gradient is its own neighbour**, so it paints all the way around its
  loop unchanged.

## Colour

- Colours are **specified in OKLCH** (lightness, chroma, hue), plus alpha.
- **All blending is done in OKLCH.** This covers both blending between stops
  within a gradient and blending between neighbouring gradients along `t`.
  - L and C are interpolated linearly.
  - Hue is always interpolated along the **shorter arc**.
  - If one colour has no meaningful hue (chroma ≈ 0), it takes the other colour's
    hue, so that blending to grey or white does not swing through other hues (the
    CSS Color 4 rule).
  - Alpha is interpolated linearly. L and C are interpolated **premultiplied** by
    alpha (hue is not), so that fading to transparent does not shift the colour
    (also the CSS Color 4 rule).
- The final OKLCH colour is converted OKLCH → OKLab → linear sRGB → sRGB to be
  composited and written to the PNG. Out-of-gamut colours are clipped to sRGB.
- Paths are composited behind each other in **gamma-encoded sRGB** with
  premultiplied alpha, the way browsers do it.

## Implementation approach (Rust)

### Crates

- `kdl` (v6+, KDL 2.0) and `miette`: input parsing and error messages that point
  at the line and column
- `png`: encoding (streams rows, no big intermediate image types)
- `rayon`: parallel rendering across rows / tiles
- argument parsing: `std::env::args` is enough (or `clap` if the CLI grows)
- `anyhow` / `thiserror`: error reporting with line numbers for parse errors

### Geometry precomputation (per path)

1. Solve Hobby's equations for control points: a tridiagonal system for open
   paths and a cyclic tridiagonal one for closed paths, each O(n).
2. Flatten the Béziers into a polyline within a sub-pixel tolerance. Store the
   cumulative arc length at each vertex so that the closest point's `t` is a
   cheap interpolation.
3. Build a spatial acceleration structure over the polyline segments (a uniform
   grid or BVH) to make closest-point queries fast.
4. Pre-sort the gradients by `t` and resolve each one's loop neighbours once, so
   that the per-pixel lookup is a binary search plus one lerp.

### Per-pixel work

For each path, top to bottom, and each pixel that is not yet fully opaque:

1. Find the closest point on the polyline. That gives the distance, the side
   (sign of the cross product), and whether the point is at an endpoint cap.
2. Map it to `t`:
   - for a point along a side: arc length → `[1,2)` or `[3,4)`;
   - for a point past an endpoint: angle around the endpoint → `[0,1)` or `[2,3)`.
3. Evaluate the two neighbouring gradients at the pixel's distance, then
   interpolate between them by `t`, all in OKLCH. On a closed path, a side
   without gradients is skipped.
4. Convert to sRGB and composite the result behind the existing pixel
   (premultiplied alpha).

### Performance and memory

- Work in tiles/rows in parallel with `rayon`. Keep a per-tile count of opaque
  pixels so that fully opaque tiles are skipped and rendering can stop as soon
  as the whole image is opaque.
- For paths with an implicit width, only visit pixels inside the path's bounding
  box expanded by its maximum gradient extent.
- Pixel buffer: one premultiplied RGBA buffer, as `f32` or `u16` per channel.
  This needs benchmarking: `f32` is simpler and more precise, `u16` uses half the
  memory. Convert to 8-bit when encoding the PNG.
- The OKLCH → sRGB conversion runs once per painted pixel per path (sin/cos for
  the hue, cubing, two 3×3 matrices, and the sRGB transfer curve). Keep it cheap:
  use a lookup table for the transfer curve, and skip the conversion entirely for
  fully transparent results.
- Keep the per-pixel shading function pure (geometry and gradient tables in, colour
  out) so it can later be ported to a GPU compute shader unchanged.

### Testing

- Unit tests: Hobby control points (compare against known MetaPost output),
  arc-length mapping, `t` mapping on sides and caps, gradient neighbour
  resolution (including the single-gradient and wrap-around cases), and
  compositing.
- Golden-image tests: small sample inputs compared against reference PNGs.
- Benchmarks (`criterion`): large images, many paths, many points.

## Not in scope for now

- **Anti-aliasing.** Not required. Hard edges (implicit path edges, the seam
  between the two sides) render aliased.
- GPU rendering (but see "Performance and memory").
- Hobby options: per-point tension, end curl, explicit directions.
- Hue interpolation directions other than the shorter arc.
- Perceptual gamut mapping (out-of-gamut colours are simply clipped).

## Input file format: KDL

The input is a [KDL 2.0](https://kdl.dev) document. KDL was chosen because it
is structured like JSON but merges well in Git: every point and stop is on its
own line, and there are no separating commas or stacked closing brackets.

### Example

```kdl
image width=800 height=600

// Frontmost path: an open curve, warm on the left, cool on the right.
path {
    point 100 500
    point 400 150
    point 700 500

    gradient 1.5 {
        stop 0  0.75 0.18 40
        stop 60 0.75 0.18 40 0.0
    }
    gradient 3.5 {
        stop 0  0.55 0.12 250
        stop 80 0.55 0.12 250 0.0
    }
}

// Behind it: a closed path, painted only on its left side, filling the rest
// of the image (its last stop is opaque).
path closed=#true {
    point 200 200
    point 600 200
    point 400 450

    gradient 1 {
        stop 0 0.95 0.02 90
    }
}
```

### Nodes

| Node       | Where           | Arguments                     | Properties                            |
| ---------- | --------------- | ----------------------------- | ------------------------------------- |
| `image`    | top level, once | —                             | `width`, `height` (integers ≥ 1, px)  |
| `path`     | top level       | —                             | `closed` (`#true` / `#false`, default `#false`) |
| `point`    | in `path`       | `x` `y` (numbers, px)         | —                                     |
| `gradient` | in `path`       | `t` (number)                  | —                                     |
| `stop`     | in `gradient`   | `distance` `L` `C` `H` [`alpha`] | —                                  |

- **`image`** must come first, before any `path`. The image starts fully
  transparent.
- **`path`** nodes are rendered in file order, the first one frontmost.
- **`point`**: the coordinates are in pixels. The origin is the top-left corner
  of the image and y points down. Pixel centres are at `+0.5`, so the pixel in
  column 3, row 7 is sampled at `(3.5, 7.5)`. A path needs at least 2 points.
  Points may lie outside the image.
- **`gradient`**: a colour ramp at position `t` around the path.
  - Open paths: `0 ≤ t < 4`; other values are an error.
  - Closed paths: gradients outside `[1, 2)` and `[3, 4)` are ignored, with a
    warning.
  - Gradients may be listed in any order; they are sorted by `t`. Several
    gradients at the same `t` keep their file order and make a hard switch at
    that position.
  - A gradient needs at least one stop.
- **`stop`**: an OKLCH colour at a distance from the path.
  - `distance` is in pixels and ≥ 0. Distances must not decrease within a
    gradient; equal distances make a hard edge.
  - `L` is lightness (0–1), `C` is chroma (≥ 0), `H` is hue in degrees (any
    value, taken modulo 360), and `alpha` is 0–1 and optional (default 1).
  - Pixels closer than the first stop get the first stop's colour. Pixels beyond
    the last stop get the last stop's colour, so a transparent last stop gives
    the path an implicit width.
- Values are positional to keep stops short. Named properties
  (`l=… c=… h=… a=…`) could be added later if wanted.

### Errors

- Unknown nodes, arguments or properties, missing required values, and values
  out of range are errors.
- Errors report the line and column, using the `kdl` crate's source spans and
  `miette` diagnostics.
- A path with no (usable) gradients paints nothing. This gives a warning, not an
  error.
