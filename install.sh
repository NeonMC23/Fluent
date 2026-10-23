#!/usr/bin/env bash
#
# Build Fluent and install it in a self-contained folder in the user's home
# directory:
#
#   ~/.fluent/bin/fluent      executable (fluent.exe on Windows)
#   ~/.fluent/config.json     configuration, with model paths relative to ~/.fluent
#   ~/.fluent/models/         local NLLB-200 model files
#
# Existing model folders from ./models are hard-linked when possible and copied
# otherwise, so the installed application never depends on the build directory.

set -euo pipefail

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
SOURCE_DIR="$SCRIPT_DIR"
cd "$SOURCE_DIR"

: "${HOME:?HOME is not set; cannot choose an installation directory}"

usage() {
    cat <<'EOF'
Usage: ./install.sh [options]

Build the Fluent desktop application and install it in ~/.fluent (or the
platform equivalent on Windows).

Options:
  --cuda             Build with NVIDIA CUDA support (requires the CUDA toolkit)
  --metal            Build with Apple Metal support (macOS only)
  --skip-build       Install the existing target/release binary
  --skip-models      Do not copy/link models from ./models
  --install-dir DIR  Install to DIR instead of ~/.fluent
  -h, --help         Show this help

Environment:
  FLUENT_INSTALL_DIR  Alternative installation directory
  FLUENT_MODELS_SRC   Directory containing the model folders (default: ./models)

The script creates:
  <install-dir>/bin/fluent[.exe]
  <install-dir>/config.json
  <install-dir>/models/nllb-200-1.3b/
  <install-dir>/models/nllb-200-3.3b/
EOF
}

die() {
    echo "Error: $*" >&2
    exit 1
}

sed_in_place() {
    local file="$1"
    local expression="$2"
    case "$(uname -s)" in
        Darwin*)
            # BSD/macOS sed requires an explicit backup suffix.
            sed -i '' "$expression" "$file"
            ;;
        *)
            sed -i "$expression" "$file"
            ;;
    esac
}

FEATURES=""
DO_BUILD=1
DO_MODELS=1
INSTALL_DIR=""

while [ "$#" -gt 0 ]; do
    case "$1" in
        --cuda)
            FEATURES="cuda"
            shift
            ;;
        --metal)
            FEATURES="metal"
            shift
            ;;
        --skip-build)
            DO_BUILD=0
            shift
            ;;
        --skip-models)
            DO_MODELS=0
            shift
            ;;
        --install-dir)
            [ "$#" -ge 2 ] || die "--install-dir requires a path"
            INSTALL_DIR="$2"
            shift 2
            ;;
        --install-dir=*)
            INSTALL_DIR="${1#*=}"
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            die "unknown option: $1 (use --help for usage)"
            ;;
    esac
done

case "$(uname -s)" in
    Linux*)
        PLATFORM="Linux"
        DEFAULT_INSTALL_DIR="$HOME/.fluent"
        EXE_SUFFIX=""
        ;;
    Darwin*)
        PLATFORM="macOS"
        DEFAULT_INSTALL_DIR="$HOME/.fluent"
        EXE_SUFFIX=""
        ;;
    MINGW*|MSYS*|CYGWIN*)
        PLATFORM="Windows"
        DEFAULT_INSTALL_DIR="$HOME/.fluent"
        EXE_SUFFIX=".exe"
        ;;
    *)
        die "unsupported operating system: $(uname -s)"
        ;;
esac

INSTALL_DIR="${INSTALL_DIR:-${FLUENT_INSTALL_DIR:-$DEFAULT_INSTALL_DIR}}"

if [ "$FEATURES" = "metal" ] && [ "$PLATFORM" != "macOS" ]; then
    die "--metal is only supported on macOS"
fi
if [ "$FEATURES" = "cuda" ] && [ "$PLATFORM" = "macOS" ]; then
    die "--cuda is not supported on macOS; use --metal instead"
fi

case "$INSTALL_DIR" in
    "~/"*)
        INSTALL_DIR="$HOME/${INSTALL_DIR#\~/}"
        ;;
    "~")
        INSTALL_DIR="$HOME"
        ;;
esac

[ -n "$INSTALL_DIR" ] || die "installation directory is empty"

if [ "$DO_BUILD" -eq 1 ]; then
    command -v cargo >/dev/null 2>&1 || die \
        "cargo was not found. Install Rust from https://rustup.rs and try again."
    echo "Building Fluent for $PLATFORM..."
    if [ -n "$FEATURES" ]; then
        cargo build --release -p fluent-gui --features "$FEATURES"
    else
        cargo build --release -p fluent-gui
    fi
fi

BIN_NAME="fluent$EXE_SUFFIX"
BIN_SRC="$SOURCE_DIR/target/release/$BIN_NAME"
[ -f "$BIN_SRC" ] || die "binary not found: $BIN_SRC (run without --skip-build)"

mkdir -p "$INSTALL_DIR/bin" "$INSTALL_DIR/models"

echo "Installing Fluent into $INSTALL_DIR..."
if command -v install >/dev/null 2>&1; then
    install -m 0755 "$BIN_SRC" "$INSTALL_DIR/bin/$BIN_NAME"
else
    cp "$BIN_SRC" "$INSTALL_DIR/bin/$BIN_NAME"
    chmod +x "$INSTALL_DIR/bin/$BIN_NAME"
fi

# Keep the installation self-contained: model paths in the installed config are
# relative to the config file, which lives next to the models/ folder.
CONFIG_SRC="$SOURCE_DIR/config.json"
CONFIG_DST="$INSTALL_DIR/config.json"
if [ -f "$CONFIG_SRC" ]; then
    cp "$CONFIG_SRC" "$CONFIG_DST"
    sed_in_place "$CONFIG_DST" \
        's#nllb-200-1\.3B#nllb-200-1.3b#g; s#nllb-200-3\.3B#nllb-200-3.3b#g'
else
    cat > "$CONFIG_DST" <<'JSON'
{
  "default_source_language": "auto",
  "default_target_language": "eng_Latn",
  "default_mode": "fast",
  "theme": "system",
  "models": {
    "fast": "models/nllb-200-1.3b",
    "accurate": "models/nllb-200-3.3b"
  },
  "generation": {
    "max_length": 512,
    "num_beams": 4
  }
}
JSON
fi

FAST_MODEL_DIR="$INSTALL_DIR/models/nllb-200-1.3b"
ACCURATE_MODEL_DIR="$INSTALL_DIR/models/nllb-200-3.3b"
mkdir -p "$FAST_MODEL_DIR" "$ACCURATE_MODEL_DIR"

install_model() {
    local target_name="$1"
    shift
    local target="$INSTALL_DIR/models/$target_name"
    local source=""

    if [ -n "$(find "$target" -mindepth 1 -maxdepth 1 -print -quit 2>/dev/null)" ]; then
        echo "  $target_name: already present"
        return 0
    fi

    for candidate in "$@"; do
        if [ -d "$candidate" ] &&
            [ -n "$(find "$candidate" -mindepth 1 -maxdepth 1 -print -quit 2>/dev/null)" ]; then
            source="$candidate"
            break
        fi
    done

    if [ -z "$source" ]; then
        echo "  $target_name: no files found (put the model files in $target)"
        return 0
    fi

    echo "  $target_name: installing from $source"
    if cp -al "$source/." "$target/" 2>/dev/null; then
        echo "    hard-linked (no extra disk space used)"
    else
        echo "    hard links unavailable; copying model files (this may take a while)..."
        find "$target" -mindepth 1 -maxdepth 1 -exec rm -rf -- {} +
        cp -a "$source/." "$target/"
    fi
}

if [ "$DO_MODELS" -eq 1 ]; then
    MODELS_SRC="${FLUENT_MODELS_SRC:-$SOURCE_DIR/models}"
    if [ -d "$MODELS_SRC" ]; then
        install_model "nllb-200-1.3b" \
            "$MODELS_SRC/nllb-200-1.3b" "$MODELS_SRC/nllb-200-1.3B"
        install_model "nllb-200-3.3b" \
            "$MODELS_SRC/nllb-200-3.3b" "$MODELS_SRC/nllb-200-3.3B"
    else
        echo "  No model folder found at $MODELS_SRC."
        echo "  Place the NLLB-200 model folders in $INSTALL_DIR/models/."
    fi
fi

cat <<EOF

Fluent installed successfully.

  Platform : $PLATFORM
  Binary   : $INSTALL_DIR/bin/$BIN_NAME
  Models   : $INSTALL_DIR/models
  Config   : $INSTALL_DIR/config.json

Run it with:

  "$INSTALL_DIR/bin/$BIN_NAME"

Optional: make the command available on your PATH
  mkdir -p "\$HOME/.local/bin"
  ln -sf "$INSTALL_DIR/bin/$BIN_NAME" "\$HOME/.local/bin/fluent"
EOF
