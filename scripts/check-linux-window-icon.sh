#!/usr/bin/env bash
#
# Verify the runtime X11 _NET_WM_ICON property against the canonical RGBA
# application mark. Run inside an X server (for example under xvfb-run).
#
# Usage:
#   xvfb-run -a ./scripts/check-linux-window-icon.sh target/release/fluent

set -euo pipefail

ROOT="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="${1:-$ROOT/target/release/fluent}"
ICON="${2:-$ROOT/src/logo/generated/window/fluent-256.rgba}"

[ -x "$BIN" ] || { echo "error: executable not found: $BIN" >&2; exit 2; }
[ -f "$ICON" ] || { echo "error: canonical icon not found: $ICON" >&2; exit 2; }
for tool in xdotool xprop python3; do
    command -v "$tool" >/dev/null 2>&1 || {
        echo "error: $tool is required" >&2
        exit 2
    }
done

TMP="$(mktemp -d "${TMPDIR:-/tmp}/fluent-icon-test.XXXXXX")"
PID=""
cleanup() {
    if [ -n "$PID" ] && kill -0 "$PID" 2>/dev/null; then
        kill "$PID" 2>/dev/null || true
        wait "$PID" 2>/dev/null || true
    fi
    rm -rf -- "$TMP"
}
trap cleanup EXIT

mkdir -p "$TMP/home/.config" "$TMP/home/.local/share"
HOME="$TMP/home" \
XDG_CONFIG_HOME="$TMP/home/.config" \
XDG_DATA_HOME="$TMP/home/.local/share" \
XDG_STATE_HOME="$TMP/home/.local/state" \
LIBGL_ALWAYS_SOFTWARE=1 \
    "$BIN" >"$TMP/app.log" 2>&1 &
PID=$!

WINDOW=""
for _ in $(seq 1 40); do
    WINDOW="$(xdotool search --name 'Fluent' 2>/dev/null | head -1 || true)"
    [ -n "$WINDOW" ] && break
    sleep 0.5
done
if [ -z "$WINDOW" ]; then
    cat "$TMP/app.log" >&2 || true
    echo "FAIL: Fluent window did not appear" >&2
    exit 1
fi

xprop -len 2000000 -id "$WINDOW" -f _NET_WM_ICON 32c _NET_WM_ICON >"$TMP/icon.txt"

python3 - "$TMP/icon.txt" "$ICON" <<'PY'
import re
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text(encoding="utf-8")
canonical = Path(sys.argv[2]).read_bytes()
if "=" not in text:
    raise SystemExit("FAIL: no _NET_WM_ICON property")
values = [int(value) for value in re.findall(r"\b\d+\b", text.split("=", 1)[1])]
if len(values) < 2:
    raise SystemExit("FAIL: malformed _NET_WM_ICON property")
width, height = values[:2]
pixels = values[2:]
if (width, height) != (256, 256):
    raise SystemExit(f"FAIL: unexpected window icon size {width}x{height}")
if len(pixels) != width * height:
    raise SystemExit("FAIL: incomplete _NET_WM_ICON data")

def pixel_offset(index: int) -> int:
    return index * 4

for index, expected in enumerate(
    (
        (canonical[offset + 3] << 24)
        | (canonical[offset] << 16)
        | (canonical[offset + 1] << 8)
        | canonical[offset + 2]
    )
    for offset in range(0, len(canonical), 4)
):
    if pixels[index] != expected:
        raise SystemExit(
            f"FAIL: window icon pixel {index} differs from the canonical mark"
        )

print("window icon: OK (256x256 _NET_WM_ICON matches canonical RGBA mark)")
PY
