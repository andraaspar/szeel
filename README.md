# szeel

Renders PNG images from a text file that describes them as gradient-coloured
paths built with Hobby's algorithm.

> **Status:** early development. Paths, groups, masks and reuse are
> implemented on the CPU. See `examples/` for sample inputs.

## Prerequisites

- **Rust** (stable), installed with [rustup](https://rustup.rs):

  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  ```

  This installs into your home directory and does not need `sudo`. Open a new
  terminal afterwards, or run `. "$HOME/.cargo/env"`.

- **A C linker** (`cc`), which Rust uses to link the final binary. Most Linux
  systems have one. If yours doesn't, install it with your package manager, for
  example `sudo apt install build-essential` on Debian or Ubuntu, or
  `sudo dnf install gcc` on Fedora.

All Rust dependencies are downloaded automatically by Cargo on the first build.

## Building

Release build (optimised; use this for real rendering):

```bash
cargo build --release
```

The binary is written to `target/release/szeel`.

Debug build (faster to compile, much slower to run):

```bash
cargo build
```

The binary is written to `target/debug/szeel`.

## Running

```bash
target/release/szeel <input.kdl> <output.png>
```

Or build and run in one step:

```bash
cargo run --release -- <input.kdl> <output.png>
```

## Input format

A scene is a [KDL](https://kdl.dev) file. It starts with
`image width=… height=…`, followed by what to draw, **front first**: each
element is composited behind everything drawn before it.

| Node       | Where                                  | Contents                                                     |
| ---------- | -------------------------------------- | ------------------------------------------------------------ |
| `image`    | first, top level only                  | `width`, `height` properties                                 |
| `path`     | top level, `group`, `mask`, `defs`     | `point x y`, `gradient t { stop … }`, at most one `mask`; `closed`, `id` properties |
| `group`    | top level, `group`, `mask`, `defs`     | `path`, `group`, `use`, at most one `mask`; `id` property    |
| `use "id"` | top level, `group`, `mask`             | at most one `mask`                                           |
| `mask`     | in `path`, `group`, `use` or `mask`    | `path`, `group`, `use`, at most one `mask`; `invert` property |
| `defs`     | top level, any number of times         | `path`, `group`                                              |

- **Paths.** The points are joined by a smooth curve through all of them
  (Hobby's algorithm). Coordinates are in pixels, with y pointing down. A
  `closed=#true` path loops back to its first point. Each pixel is coloured
  from its closest point on the path.
- **Gradients.** `gradient t { … }` places a colour ramp at position `t` around
  the path. A pixel blends the two gradients on either side of its own `t`,
  wrapping around. On an open path, `t` runs from 0 to 4: 0–1 is the cap
  around the start point, 1–2 is the left side from start to end, 2–3 is the
  cap around the end point, and 3–4 is the right side from end back to start.
  Left and right are as seen on screen when travelling along the path. On a
  closed path only the sides exist: 1–2 (left) and 3–4 (right). If the points
  go clockwise on screen, the inside is the right side.
- **Stops.** `stop distance L C H [alpha]` gives an OKLCH colour at a distance
  in pixels from the path. Distances must not decrease, and alpha defaults
  to 1. The last stop's colour continues forever: end with alpha 0 for a glow
  of limited width, or opaque to fill out to the image edges.
- **Groups** collect paths and other groups. On its own a group changes
  nothing; it matters when it has a mask.
- **Masks.** Only a mask's alpha counts (stop colours are still written in
  full). The masked element is scaled by the mask's alpha, or by one minus it
  with `invert=#true`. A masked group is drawn as a whole first, then scaled.
  Combine masks by nesting: several shapes in one mask are a union, a `mask`
  inside a `mask` is an intersection, and an inverted mask inside a mask
  subtracts.
- **Reuse.** Give a `path` or `group` an `id` (unique in the file) and draw it
  again with `use "id"`, in the image or inside masks. A `use` can have a mask
  of its own. Only elements that come earlier in the file can be used.
  Elements inside `defs` are only drawn where they are used.
- **Limits.** Counting every `use` and mask in full, a file may draw at most
  1,000,000 paths, and groups, masks and `use`s may nest at most 64 levels
  deep.

See `examples/mask.kdl` and `examples/reuse.kdl`.

## Using szeel from Claude

[`skill/`](skill/) holds a skill that lets Claude make images with szeel in
regular claude.ai chats, including in the mobile app. The skill bundles a
static Linux binary, so Claude runs it in its code-execution sandbox. Claude
writes a scene, renders it, looks at the result, and refines the scene before
handing over the PNG.

To build the skill package:

```bash
skill/package.sh
```

This writes `target/szeel-skill.zip`. Upload it to claude.ai under
Settings → Capabilities → Skills, with code execution turned on. Run the
script and upload the zip again after any change to szeel or to
[`skill/SKILL.md`](skill/SKILL.md).

`SKILL.md` is also a more detailed guide to the input format, with recipes.

## Testing

```bash
cargo test
```

The golden-image test renders every `examples/*.kdl` file and compares it with
`tests/golden/<name>.png`. After an intended change to the output, regenerate
the reference images and check them by eye before committing:

```bash
UPDATE_GOLDEN=1 cargo test --release --test golden
```

## Benchmarks

```bash
cargo bench
```
