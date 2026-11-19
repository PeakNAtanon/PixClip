#!/usr/bin/env bash
set -euo pipefail

project="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project"
version="$(sed -n 's/^version = "\([0-9][0-9.]*\)"$/\1/p' Cargo.toml)"
case "$(uname -s)" in
    Linux) platform=linux ;;
    Darwin) platform=macos ;;
    *) echo 'Build this archive on Linux or macOS.' >&2; exit 1 ;;
esac
case "$(uname -m)" in
    x86_64) architecture=x64 ;;
    aarch64|arm64) architecture=arm64 ;;
    *) echo 'Unsupported architecture.' >&2; exit 1 ;;
esac

binary="${1:-target/release/pixclip}"
test -x "$binary"
staging="$(mktemp -d -t pixclip-package.XXXXXX)"
trap 'rm -rf "$staging"' EXIT
mkdir -p "$staging/PixClip/docs" dist
install -m 755 "$binary" "$staging/PixClip/pixclip-bin"
install -m 755 PixClip "$staging/PixClip/PixClip"
install -m 644 README.md LICENSE "$staging/PixClip"
install -m 644 assets/fonts/OFL.txt "$staging/PixClip/LICENSE-NotoSansThai.txt"
install -m 644 docs/README-PixClip.md "$staging/PixClip/docs"

archive="dist/PixClip-$version-$platform-$architecture.tar.gz"
COPYFILE_DISABLE=1 tar -czf "$archive" -C "$staging" PixClip
tar -tzf "$archive" >/dev/null
echo "$archive"
