#!/usr/bin/env bash
set -euo pipefail
PROJECT_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$PROJECT_DIR"
command -v cargo >/dev/null 2>&1 || { echo 'Install Rust from https://rustup.rs first.' >&2; exit 1; }
cargo build --release --locked
cp target/release/pixclip pixclip-bin
chmod +x pixclip-bin PixClip
echo 'Built pixclip-bin. Run ./PixClip or ./PixClip --cli.'
