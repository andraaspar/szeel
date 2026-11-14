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
