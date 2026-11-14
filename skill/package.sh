#!/usr/bin/env bash
# Builds a static Linux binary and packages it with SKILL.md as a skill zip
# for uploading to claude.ai (Settings → Capabilities → Skills).
# Output: target/szeel-skill.zip
set -euo pipefail
cd "$(dirname "$0")/.."

target=x86_64-unknown-linux-musl
rustup target add "$target" >/dev/null
cargo build --release --target "$target"

stage=target/skill-stage
rm -rf "$stage"
mkdir -p "$stage/szeel/bin"
cp skill/SKILL.md "$stage/szeel/"
cp "target/$target/release/szeel" "$stage/szeel/bin/"
strip "$stage/szeel/bin/szeel" 2>/dev/null || true

rm -f target/szeel-skill.zip
(cd "$stage" && zip -qr ../szeel-skill.zip szeel)
echo "Wrote target/szeel-skill.zip"
