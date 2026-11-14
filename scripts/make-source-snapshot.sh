#!/usr/bin/env bash
#
# Create a clean source snapshot from the committed HEAD.
#
# The archive contains only tracked files (no .git, target/, dist/, local
# models or untracked files). It is deterministic for a given commit, so it is
# a low-risk alternative to publishing full Git history while historical
# third-party assets remain in that history.

set -euo pipefail

ROOT="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
DIST="$ROOT/dist"
VERSION="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$ROOT/Cargo.toml" | head -n 1)"
[ -n "$VERSION" ] || { echo "error: could not read workspace version" >&2; exit 1; }

if ! git -C "$ROOT" diff --quiet || ! git -C "$ROOT" diff --cached --quiet; then
    echo "warning: working tree has uncommitted changes; they will NOT be included" >&2
fi
if [ -n "$(git -C "$ROOT" status --porcelain --untracked-files=no)" ]; then
    echo "warning: tracked files differ from HEAD; snapshot uses HEAD only" >&2
fi

mkdir -p "$DIST"
OUT="$DIST/fluent-$VERSION-source.tar.gz"
git -C "$ROOT" archive --format=tar.gz -o "$OUT" HEAD
(
    cd "$DIST"
    sha256sum "$(basename "$OUT")" > "$(basename "$OUT").sha256"
)

echo "Source snapshot: $OUT"
echo "SHA-256: $(cut -d' ' -f1 "$OUT.sha256")"
