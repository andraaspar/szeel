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

To reformat scene files in place, one node per line with four-space indents,
keeping comments and blank lines between nodes:

```bash
target/release/szeel fmt <file.kdl>...
```

With `--check` it changes nothing, lists the files that need formatting, and
exits with an error if there are any.

## Input format

A scene is a [KDL](https://kdl.dev) file. It starts with
`image width=… height=…`, followed by what to draw, **back first**, like
painting: each element is drawn over everything before it. (The renderer
still works front to back internally, so it can skip whatever is hidden.)

| Node       | Where                                  | Contents                                                     |
| ---------- | -------------------------------------- | ------------------------------------------------------------ |
| `image`    | first, top level only                  | `width`, `height` properties                                 |
| `path`     | top level, `group`, `mask`, `defs`     | `node x y`, `cusp x y`, `gradient t { stop … }`, at most one `mask` or `unmask`; `id` property |
| `loop`     | same as `path`                         | same as `path`                                               |
| `group`    | top level, `group`, `mask`, `defs`     | `path`, `loop`, `group`, `use`, at most one `mask` or `unmask`; `id` property |
| `use "id"` | top level, `group`, `mask`             | at most one `mask` or `unmask`                               |
| `mask`     | in `path`, `loop`, `group`, `use` or `mask` | `path`, `loop`, `group`, `use`, at most one `mask` or `unmask` |
| `unmask`   | same as `mask`                         | same as `mask`                                               |
| `defs`     | top level, any number of times         | `path`, `loop`, `group`                                      |

- **Paths.** The nodes (`node x y`) are joined by a smooth curve through all
  of them (Hobby's algorithm). Coordinates are in pixels, with y pointing
  down. A `loop` is a closed path: it joins its last node back to its first.
  Each pixel is coloured from its closest point on the path.
- **Cusps.** `cusp x y` is a node where the curve may turn sharply: the path
  is cut there, and each stretch between cusps is a smooth curve of its own
  (like MetaPost's `&`). A stretch of two nodes is straight, so a `loop` of
  cusps only is a polygon. On an open path, a cusp as the first or last node
  does nothing. See `examples/cusp.kdl`.
- **Gradients.** `gradient t { … }` places a colour ramp at position `t` around
  the path. A pixel blends the two gradients on either side of its own `t`,
  wrapping around. On an open path, `t` runs from 0 to 4: 0–1 is the cap
  around the start point, 1–2 is the left side from start to end, 2–3 is the
  cap around the end point, and 3–4 is the right side from end back to start.
  Left and right are as seen on screen when travelling along the path. On a
  `loop` only the sides exist: 1–2 (left) and 3–4 (right). If the nodes
  go clockwise on screen, the inside is the right side.
- **Stops.** `stop distance L C H [alpha] [ease=…] [target=n]` gives an OKLCH colour at a distance
  in pixels from the path. Distances must not decrease, and alpha defaults
  to 1. The last stop's colour continues forever: end with alpha 0 for a glow
  of limited width, or opaque to fill out to the image edges.
- **Tweening.** Between one gradient and the next along `t`, the stops of
  the two are paired up, and each pair moves from the first gradient's stop
  to the second's, the colour turning from one to the other. `target=n`
  pairs a stop with stop `n` (counting from 0) of the next gradient; without
  it, a stop pairs with the stop with its own index. An index past the next
  gradient's last stop means that last stop, so several stops can merge into
  one, and an index below the previous stop's target means that target, so
  stops never cross. A stop of the next gradient that nothing targets pairs
  by the same rule (its own index, or the last stop), held between the stops
  whose targets lie either side of it. How far the stops have moved is set
  by the gradient's easing. Targets only apply going forward along `t`; the
  way back round the loop uses the other gradient's targets.
- **Easing.** `ease=…` on a stop sets how the colour moves from that stop
  to the next one; on a gradient (`gradient t ease=… { … }`) it sets how
  the colour moves from that gradient to the next one along `t`. Either takes
  `no` (the default, CSS `linear`), `yes`, `in`, `out` or `in-out` (CSS
  `ease`, `ease-in`, `ease-out` and `ease-in-out`), or one of the CSS easing
  functions `"cubic-bezier(x1, y1, x2, y2)"`,
  `"steps(n[, jump-start|jump-end|jump-none|jump-both|start|end])"`,
  `step-start` or `step-end`. Values with parentheses need quotes. Gradients
  blend in a loop that wraps round, so
  the last gradient's easing leads back to the first. On an open path the
  loop is the whole of `t`; on a `loop` each side is a loop of its own,
  and gradients never blend across sides. Easing on the last stop, or on a
  gradient that is alone in its loop (the only one on a path or on a side),
  does nothing. A `cubic-bezier` that overshoots is clamped, so the blend
  never goes past either end's colour.
- **Groups** collect paths and other groups. On its own a group changes
  nothing; it matters when it has a mask.
- **Masks.** Only a mask's alpha counts (stop colours are still written in
  full). The masked element is scaled by the mask's alpha; an `unmask` is an
  inverted mask, which scales it by one minus the alpha. A masked group is
  drawn as a whole first, then scaled.
  Combine masks by nesting: several shapes in one mask are a union, a `mask`
  inside a `mask` is an intersection, and an `unmask` inside a mask
  subtracts.
- **Reuse.** Give a `path` or `group` an `id` (unique in the file) and draw it
  again with `use "id"`, in the image or inside masks. A `use` can have a mask
  of its own. Only elements that come earlier in the file can be used, so
  something painted over a shape can use that shape as its mask.
  Elements inside `defs` are only drawn where they are used.
- **Limits.** Counting every `use` and mask in full, a file may draw at most
  1,000,000 paths, and groups, masks and `use`s may nest at most 64 levels
  deep.

See `examples/mask.kdl` and `examples/reuse.kdl`.

## Editor

`web/` holds a desktop editor for scenes, built with Tauri 2: the KDL text on
the left, a live render on the right, with the paths drawn over it. Drag
nodes to move them, double-click a curve to add a node, Alt+click a node
to delete it, and double-click a node (or select it and press C) to switch
it between `node` and `cusp`; cusps show as diamonds. Every change is written back into the
text. Edit → Format Document (Ctrl+Shift+F) lays the text out one node per
line with four-space indents, keeping comments and blank lines between nodes.

The UI (`web/src`, Svelte) does the editing; the Rust side
(`web/src-tauri`) parses and renders with the `szeel` crate, natively and in
parallel, and handles files: open, save, save as, recent files, opening a
file named on the command line or dropped on the window, PNG export straight
to disk, and reloading the open file when another program changes it. It asks
before dropping unsaved changes.

Besides the prerequisites above it needs Node.js 20 or later and the WebKitGTK
development libraries. On Fedora:

```bash
sudo dnf install webkit2gtk4.1-devel libsoup3-devel javascriptcoregtk4.1-devel openssl-devel librsvg2-devel
```

On Debian or Ubuntu:

```bash
sudo apt install libwebkit2gtk-4.1-dev libsoup-3.0-dev libjavascriptcoregtk-4.1-dev libssl-dev librsvg2-dev
```

Then:

```bash
cd web
npm install
npm run dev                       # opens the editor, reloading as you change the code
npm run dev -- -- -- scene.kdl    # the same, opening scene.kdl
npm run build                     # .rpm and .deb in target/release/bundle/
```

The built binary is `target/release/szeel-editor`; give it a `.kdl` file to
open that.

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
