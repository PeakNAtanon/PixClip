#!/usr/bin/env bash
set -euo pipefail

REPO_SLUG="${1:-}"
BRANCH="${2:-main}"
INSTALL_DIR="${PIXCLIP_INSTALL_DIR:-${HOME}/.local/share/PixClip}"
COMMAND_DIR="${PIXCLIP_BIN_DIR:-${HOME}/.local/bin}"

if [[ -z "$REPO_SLUG" || ! "$REPO_SLUG" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]]; then
    echo "Usage: curl -fsSL https://raw.githubusercontent.com/OWNER/REPO/main/install.sh | bash -s -- OWNER/REPO" >&2
    exit 2
fi
if [[ ! "$BRANCH" =~ ^[A-Za-z0-9_./-]+$ || "$BRANCH" == *..* ]]; then
    echo "Invalid branch name: $BRANCH" >&2
    exit 2
fi

for command in curl tar python3; do
    if ! command -v "$command" >/dev/null 2>&1; then
        echo "Required command was not found: $command" >&2
        exit 1
    fi
done

TEMP_DIR="$(mktemp -d -t pixclip-install.XXXXXX)"
cleanup() {
    rm -rf "$TEMP_DIR"
}
trap cleanup EXIT

ARCHIVE="$TEMP_DIR/PixClip.tar.gz"
ARCHIVE_URL="https://github.com/${REPO_SLUG}/archive/refs/heads/${BRANCH}.tar.gz"
echo "Downloading PixClip from ${REPO_SLUG} (${BRANCH})..."
curl -fsSL "$ARCHIVE_URL" -o "$ARCHIVE"
tar -xzf "$ARCHIVE" -C "$TEMP_DIR"
SOURCE_DIR="$(find "$TEMP_DIR" -mindepth 1 -maxdepth 1 -type d -print -quit)"
if [[ -z "$SOURCE_DIR" || ! -f "$SOURCE_DIR/media_toolkit.py" ]]; then
    echo "The downloaded archive does not contain a valid PixClip source tree." >&2
    exit 1
fi

mkdir -p "$INSTALL_DIR"
cp -a "$SOURCE_DIR/." "$INSTALL_DIR/"
chmod +x "$INSTALL_DIR/PixClip" "$INSTALL_DIR/INSTALL-PIXCLIP-COMMAND.sh" "$INSTALL_DIR/install.sh"

echo "Installing PixClip tools and Linux GUI dependency..."
python3 "$INSTALL_DIR/Install-Media-Tools.py"
bash "$INSTALL_DIR/INSTALL-PIXCLIP-COMMAND.sh"

echo
echo "PixClip installed at: $INSTALL_DIR"
echo "Run it with: $COMMAND_DIR/PixClip"
if [[ ":${PATH}:" != *":${COMMAND_DIR}:"* ]]; then
    echo "Add the command directory to PATH with:"
    echo '  export PATH="$HOME/.local/bin:$PATH"'
fi
