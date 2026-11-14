#!/usr/bin/env bash
#
# Print the single workspace version from Cargo.toml.
# Build scripts and workflows use this instead of duplicating a constant.

set -euo pipefail
ROOT="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$ROOT/Cargo.toml" | head -n 1)"
[ -n "$VERSION" ] || { echo "error: could not read workspace version" >&2; exit 1; }
printf '%s\n' "$VERSION"
