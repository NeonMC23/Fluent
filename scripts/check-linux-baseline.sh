#!/usr/bin/env bash
#
# Verify the maximum GLIBC symbol required by a Linux executable.
#
# Usage: ./scripts/check-linux-baseline.sh <binary> [max-version]
# Default max-version: 2.35 (Ubuntu 22.04 baseline).

set -euo pipefail

BIN="${1:-}"
ALLOWED="${2:-2.35}"

if [ -z "$BIN" ] || [ ! -f "$BIN" ]; then
    echo "usage: $0 <binary> [max-version]" >&2
    exit 2
fi
command -v objdump >/dev/null 2>&1 || {
    echo "error: objdump not found" >&2
    exit 2
}

VERSIONS="$(objdump -T "$BIN" 2>/dev/null |
    grep -oE 'GLIBC_[0-9.]+' | sed 's/^GLIBC_//' | sort -Vu || true)"
if [ -z "$VERSIONS" ]; then
    echo "error: no GLIBC symbol versions found in $BIN" >&2
    exit 1
fi

MAX="$(printf '%s\n' "$VERSIONS" | tail -n 1)"
HIGHEST="$(printf '%s\n%s\n' "$ALLOWED" "$MAX" | sort -V | tail -n 1)"
if [ "$HIGHEST" != "$ALLOWED" ]; then
    echo "FAIL: $BIN requires GLIBC_$MAX, baseline is GLIBC_$ALLOWED" >&2
    exit 1
fi

CXX="$(objdump -T "$BIN" 2>/dev/null |
    grep -oE 'GLIBCXX_[0-9.]+|CXXABI_[0-9.]+' | sort -Vu | tail -n 1 || true)"
if [ -n "$CXX" ]; then
    echo "warning: binary also requires $CXX (libstdc++)" >&2
fi

echo "OK: $BIN requires at most GLIBC_$MAX (baseline $ALLOWED)"
