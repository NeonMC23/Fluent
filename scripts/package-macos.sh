#!/usr/bin/env bash
#
# Stage a minimal Fluent.app bundle. This is resource/bundle preparation, not
# signing, notarization or a distributable release.
#
# Run on macOS after a real build for a usable app. On other hosts, --binary
# can be used to validate the layout only; a non-macOS executable is not a
# usable macOS application.

set -euo pipefail

ROOT="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
DIST="${FLUENT_DIST_DIR:-$ROOT/dist}"
BINARY_PATH=""
DO_BUILD=1

usage() {
    cat <<'USAGE'
Usage: ./scripts/package-macos.sh [--binary FILE] [--skip-build] [--output DIR]

Stages dist/Fluent.app/Contents/{MacOS/fluent,Resources/Fluent.icns,Info.plist}.
No signing or notarization is performed. Models and configuration are not
included.
USAGE
}

die() { printf 'Error: %s\n' "$*" >&2; exit 1; }

while [ "$#" -gt 0 ]; do
    case "$1" in
        --binary) [ "$#" -ge 2 ] || die "--binary requires a file"; BINARY_PATH="$2"; shift 2 ;;
        --binary=*) BINARY_PATH="${1#*=}"; shift ;;
        --skip-build) DO_BUILD=0; shift ;;
        --output) [ "$#" -ge 2 ] || die "--output requires a directory"; DIST="$2"; shift 2 ;;
        --output=*) DIST="${1#*=}"; shift ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown option: $1" ;;
    esac
done

VERSION="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$ROOT/Cargo.toml" | head -n 1)"
[ -n "$VERSION" ] || die "could not read workspace version"
[ -n "$DIST" ] && [ "$DIST" != "/" ] || die "refusing to use an unsafe output directory"

if [ -n "$BINARY_PATH" ]; then
    DO_BUILD=0
fi

if [ "$(uname -s)" != "Darwin" ]; then
    if [ -z "$BINARY_PATH" ]; then
        die "build macOS bundles on macOS, or pass --binary for a static layout check"
    fi
    printf 'Warning: not running on macOS; this checks bundle layout only, not a usable macOS binary.\n' >&2
fi

if [ "$DO_BUILD" -eq 1 ]; then
    command -v cargo >/dev/null 2>&1 || die "cargo was not found"
    (cd "$ROOT" && cargo build --release --locked -p fluent-gui)
fi

if [ -n "$BINARY_PATH" ]; then
    BIN_SRC="$BINARY_PATH"
else
    TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
    case "$TARGET_DIR" in /*) ;; *) TARGET_DIR="$ROOT/$TARGET_DIR" ;; esac
    BIN_SRC="$TARGET_DIR/release/fluent"
fi
[ -f "$BIN_SRC" ] || die "executable not found: $BIN_SRC"

ICNS="$ROOT/src/logo/generated/macos/Fluent.icns"
PLIST_TEMPLATE="$ROOT/crates/fluent-gui/assets/macos/Info.plist.in"
[ -f "$ICNS" ] || die "missing macOS icon: $ICNS"
[ -f "$PLIST_TEMPLATE" ] || die "missing Info.plist template: $PLIST_TEMPLATE"

mkdir -p "$DIST"
APP="$DIST/Fluent.app"
case "$APP" in
    "$ROOT/dist/"*|"$DIST/"*) ;;
    *) die "refusing to replace an unexpected bundle path: $APP" ;;
esac
rm -rf -- "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

install -m 0755 "$BIN_SRC" "$APP/Contents/MacOS/fluent"
install -m 0644 "$ICNS" "$APP/Contents/Resources/Fluent.icns"
sed "s/@VERSION@/$VERSION/g" "$PLIST_TEMPLATE" > "$APP/Contents/Info.plist"
printf 'APPL????' > "$APP/Contents/PkgInfo"

if [ "$(uname -s)" = "Darwin" ] && command -v plutil >/dev/null 2>&1; then
    plutil -lint "$APP/Contents/Info.plist" >/dev/null || die "Info.plist validation failed"
fi

echo "Staged: $APP"
echo "Version: $VERSION"
echo "No signing, notarization, models or configuration included."
