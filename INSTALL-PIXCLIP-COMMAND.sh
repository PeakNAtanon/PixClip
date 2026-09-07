#!/usr/bin/env bash
set -euo pipefail

SOURCE="${BASH_SOURCE[0]}"
while [ -L "$SOURCE" ]; do
    TARGET="$(readlink "$SOURCE")"
    [[ "$TARGET" = /* ]] || TARGET="$(dirname "$SOURCE")/$TARGET"
    SOURCE="$TARGET"
done
SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$SOURCE")" && pwd)"
COMMAND_DIR="${HOME}/.local/bin"
COMMAND_PATH="${COMMAND_DIR}/PixClip"

mkdir -p "$COMMAND_DIR"

if [ -e "$COMMAND_PATH" ] && [ ! -L "$COMMAND_PATH" ]; then
    echo "ไม่ได้ติดตั้ง: มีไฟล์อยู่แล้วที่ $COMMAND_PATH" >&2
    exit 1
fi

chmod +x "$SCRIPT_DIR/PixClip"
ln -sfn "$SCRIPT_DIR/PixClip" "$COMMAND_PATH"

echo "ติดตั้งคำสั่ง PixClip แล้ว"
if [[ ":${PATH}:" != *":${COMMAND_DIR}:"* ]]; then
    echo "เพิ่ม PATH ชั่วคราวด้วยคำสั่งนี้:"
    echo '  export PATH="$HOME/.local/bin:$PATH"'
    echo "จากนั้นใช้คำสั่ง: PixClip"
else
    echo "เรียก GUI ได้ด้วย: PixClip"
fi
