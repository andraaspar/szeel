# szeel

Renders PNG images from a text file that describes them as gradient-coloured
paths built with Hobby's algorithm.

> **Status:** early development. The command-line interface exists, but
> rendering is not implemented yet. See [plan.md](plan.md) for the design.

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

## Testing

```bash
cargo test
```
