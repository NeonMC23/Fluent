#!/usr/bin/env bash
#
# Build a portable application-only Linux archive.
#
# The archive contains the release GUI binary, the MIT LICENSE and a short
# runtime README. It deliberately contains no models, tokenizer files,
# configuration, user data, build output, or development files.
#
# The tar metadata is normalised (sorted names, fixed mtime, numeric owner).
# The Rust binary itself may still contain build-specific bytes; see
# docs/LINUX.md.

set -euo pipefail

ROOT="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
DIST="$ROOT/dist"
DO_BUILD=1
BINARY_PATH=""

usage() {
    cat <<'EOF'
Usage: ./scripts/package-linux.sh [options]

Build the release GUI and create:
  dist/fluent-<version>-linux-<arch>.tar.gz
  dist/fluent-<version>-linux-<arch>.tar.gz.sha256

Options:
  --skip-build      Use target/release/fluent without running Cargo
  --binary FILE     Use a prebuilt executable instead of building
  --output DIR      Write the archive to DIR instead of ./dist
  -h, --help        Show this help

The archive contains only bin/fluent, LICENSE and README.txt. Models must be
supplied separately; they are never packaged.
EOF
}

die() {
    printf 'Error: %s\n' "$*" >&2
    exit 1
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --skip-build)
            DO_BUILD=0
            shift
            ;;
        --binary)
            [ "$#" -ge 2 ] || die "--binary requires a file"
            BINARY_PATH="$2"
            shift 2
            ;;
        --binary=*)
            BINARY_PATH="${1#*=}"
            shift
            ;;
        --output)
            [ "$#" -ge 2 ] || die "--output requires a directory"
            DIST="$2"
            shift 2
            ;;
        --output=*)
            DIST="${1#*=}"
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            die "unknown option: $1 (use --help)"
            ;;
    esac
done

[ "$(uname -s)" = "Linux" ] || die "this packaging script targets Linux"

case "$(uname -m)" in
    x86_64|aarch64) ARCH="$(uname -m)" ;;
    *) die "unsupported Linux architecture: $(uname -m)" ;;
esac

VERSION="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$ROOT/Cargo.toml" | head -n 1)"
[ -n "$VERSION" ] || die "could not read the workspace version from Cargo.toml"

NAME="fluent-$VERSION-linux-$ARCH"
ARCHIVE="$DIST/$NAME.tar.gz"

if [ -n "$BINARY_PATH" ]; then
    DO_BUILD=0
fi

TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
case "$TARGET_DIR" in
    /*) ;;
    *) TARGET_DIR="$ROOT/$TARGET_DIR" ;;
esac

if [ -n "$BINARY_PATH" ]; then
    BIN_SRC="$BINARY_PATH"
else
    BIN_SRC="$TARGET_DIR/release/fluent"
fi

case "$ROOT" in
    *' '*) die "the checkout path contains spaces; path remapping requires a space-free path" ;;
esac

command -v tar >/dev/null 2>&1 || die "tar was not found"
command -v gzip >/dev/null 2>&1 || die "gzip was not found"
command -v sha256sum >/dev/null 2>&1 || die "sha256sum was not found"
command -v install >/dev/null 2>&1 || die "install was not found"

GLIBC_BUILD="unknown"
if command -v ldd >/dev/null 2>&1; then
    GLIBC_BUILD="$(ldd --version 2>/dev/null | head -n 1 || true)"
    [ -n "$GLIBC_BUILD" ] || GLIBC_BUILD="unknown"
fi

if [ "$DO_BUILD" -eq 1 ]; then
    command -v cargo >/dev/null 2>&1 || die "cargo was not found"
    echo "Building Fluent $VERSION for Linux/$ARCH..."
    CARGO_REGISTRY="${CARGO_HOME:-$HOME/.cargo}/registry"
    # Keep developer and registry paths out of panic locations in the binary.
    export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$ROOT=. --remap-path-prefix=$CARGO_REGISTRY=/cargo/registry --remap-path-prefix=$TARGET_DIR=/target"
    (cd "$ROOT" && cargo build --release --locked -p fluent-gui)
fi

[ -f "$BIN_SRC" ] || die "executable not found: $BIN_SRC"

mkdir -p -- "$DIST"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/fluent-package.XXXXXX")"
trap 'rm -rf -- "$WORK"' EXIT
STAGE="$WORK/$NAME"
mkdir -p "$STAGE/bin"

install -m 0755 "$BIN_SRC" "$STAGE/bin/fluent"
install -m 0644 "$ROOT/LICENSE" "$STAGE/LICENSE"

cat > "$STAGE/README.txt" <<EOF
Fluent $VERSION - portable Linux/$ARCH build
================================================

This archive contains the application only. It does NOT contain NLLB-200
models, tokenizer files, configuration, user data or installer scripts.

Runtime requirements
--------------------
- Linux on $ARCH with glibc at least as new as the build host.
  Build host runtime: $GLIBC_BUILD
  The strongest requirement can be inspected with:

      objdump -T bin/fluent | grep -o 'GLIBC_[0-9.]*' | sort -Vu | tail -1
- X11 or Wayland with an OpenGL 3.3 capable driver (eframe/glow renderer).
- fontconfig and the usual desktop libraries. Run:

      ldd bin/fluent

  to inspect the exact shared-library dependencies of this binary.

Install / run
-------------
1. Extract the archive:

       tar -xzf $NAME.tar.gz
       cd $NAME

2. Put your model directories next to this README:

       models/nllb-200-1.3b/
           config.json
           tokenizer.json
           pytorch_model.bin      (or safetensors / sharded checkpoint)
       models/nllb-200-3.3b/
           ...

   Fluent does not download these files. Their license is CC-BY-NC-4.0;
   see the project documentation. Only config.json, tokenizer.json and a
   complete checkpoint are required.

3. Run from this directory:

       ./bin/fluent

   The application searches for config.json next to the executable, in its
   parent, in the platform configuration directory, and in the current
   directory. With no config.json, it uses ./models/nllb-200-1.3b and
   ./models/nllb-200-3.3b relative to the current directory.

To use models stored elsewhere, create a config.json in this directory or set
FLUENT_CONFIG to its absolute path. Example model entries:

       "models": {
         "fast": "/absolute/path/nllb-200-1.3b",
         "accurate": "/absolute/path/nllb-200-3.3b"
       }

The GUI starts without a model; the first translation reports an actionable
missing-model error with the expected directory.

Licenses
--------
The application source is MIT-licensed (see LICENSE). NLLB-200 model and
tokenizer assets are not included and remain subject to Meta's CC-BY-NC-4.0
terms. See MODEL_PROVENANCE.md in the Fluent source repository.

Verification
------------
Verify the archive with the adjacent checksum file:

       sha256sum -c $NAME.tar.gz.sha256

Run that command from the directory containing both the archive and its
.sha256 file. A checksum only establishes integrity relative to the channel
that supplied it; it is not a signature or proof of authenticity.
EOF

# Ensure the archive cannot accidentally pick up models or local files.
if find "$STAGE" -mindepth 1 -not -path "$STAGE/bin/fluent" \
    -not -path "$STAGE/bin" -not -path "$STAGE/LICENSE" \
    -not -path "$STAGE/README.txt" -print -quit | grep -q .; then
    die "unexpected file in staging directory"
fi

echo "Creating deterministic tar metadata..."
TMP_ARCHIVE="$ARCHIVE.tmp"
tar --sort=name \
    --mtime='UTC 2020-01-01' \
    --owner=0 --group=0 --numeric-owner \
    --format=gnu \
    -C "$WORK" -cf - "$NAME" | gzip -n -9 > "$TMP_ARCHIVE"
mv -f -- "$TMP_ARCHIVE" "$ARCHIVE"

(
    cd "$DIST"
    sha256sum "$NAME.tar.gz" > "$NAME.tar.gz.sha256"
)

printf '\nArchive : %s\n' "$ARCHIVE"
printf 'Size    : %s bytes\n' "$(stat -c%s "$ARCHIVE")"
printf 'SHA-256 : %s\n' "$(cut -d' ' -f1 "$ARCHIVE.sha256")"
printf 'Checksum: %s\n' "$ARCHIVE.sha256"
