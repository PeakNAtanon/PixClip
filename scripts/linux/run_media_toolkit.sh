#!/usr/bin/env bash
set -euo pipefail
PROJECT_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
exec "$PROJECT_DIR/PixClip" "$@"
