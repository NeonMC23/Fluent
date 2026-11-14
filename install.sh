#!/usr/bin/env bash
#
# Fluent per-user installer for Linux (validated) and macOS (best effort).
#
# Layout on Linux:
#   ~/.local/lib/fluent/fluent              application payload
#   ~/.local/bin/fluent                     launcher symlink
#   ~/.config/fluent/config.json            configuration (user-owned)
#   ~/.local/share/fluent/models/           model assets (user-owned)
#   ~/.local/share/applications/io.fluent.Fluent.desktop
#
# Models are never bundled with the application and uninstall never removes
# configuration or model assets. Run with --help for all options.

set -euo pipefail

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
PROG="$(basename -- "${BASH_SOURCE[0]}")"
APP_ID="io.fluent.Fluent"
APP_EXE="fluent"

: "${HOME:?HOME is not set; cannot choose a per-user installation directory}"

usage() {
    cat <<'EOF'
Usage: ./install.sh [options]

Build and install Fluent for the current user. The default layout separates
the application from configuration and model assets:

  Linux
    application  ~/.local/lib/fluent/
    launcher     ~/.local/bin/fluent
    config       ${XDG_CONFIG_HOME:-~/.config}/fluent/config.json
    models       ${XDG_DATA_HOME:-~/.local/share}/fluent/models/
    desktop      ${XDG_DATA_HOME:-~/.local/share}/applications/io.fluent.Fluent.desktop

  macOS
    application  ~/Library/Application Support/io.fluent.Fluent/app/
    launcher     ~/.local/bin/fluent
    config       ~/Library/Application Support/io.fluent.Fluent/config.json
    models       ~/Library/Application Support/io.fluent.Fluent/models/

Options:
  --cuda                  Build with NVIDIA CUDA support (requires the CUDA toolkit)
  --metal                 Build with Metal support (macOS only)
  --skip-build            Do not run cargo; use --binary or target/release/fluent
  --binary FILE           Install a prebuilt executable instead of building
  --skip-models           Create model directories but do not install models
  --models-src DIR        Directory containing nllb-200-1.3b/ and nllb-200-3.3b/
                          (default: ./models; a legacy ~/.fluent/models is also
                          considered when the target is empty)
  --icon FILE             Install an SVG application icon
  --no-desktop            Skip the desktop entry and icon (Linux)
  --reset-config          Regenerate config.json even if one already exists
  --uninstall             Remove application files, launcher and desktop entry.
                          Configuration and models are always preserved.
  --app-dir DIR           Override the application payload directory
  --bin-dir DIR           Override the launcher directory
  --config-dir DIR        Override the configuration directory
  --models-dir DIR        Override the model directory
  --install-dir DIR       Legacy alias for --app-dir
  -h, --help              Show this help

Environment:
  FLUENT_APP_DIR, FLUENT_BIN_DIR, FLUENT_CONFIG_DIR, FLUENT_MODELS_DIR
  FLUENT_MODELS_SRC, FLUENT_INSTALL_DIR (legacy alias for FLUENT_APP_DIR)

The installer never uses sudo, never installs system packages, never
downloads models and never deletes configuration, models or unrelated files.
EOF
}

die() {
    printf 'Error: %s\n' "$*" >&2
    exit 1
}

warn() {
    printf 'Warning: %s\n' "$*" >&2
}

expand_tilde() {
    case "$1" in
        "~") printf '%s' "$HOME" ;;
        "~/"*) printf '%s/%s' "$HOME" "${1#\~/}" ;;
        *) printf '%s' "$1" ;;
    esac
}

absolute_path() {
    case "$1" in
        /*) printf '%s' "$1" ;;
        *) printf '%s/%s' "$PWD" "$1" ;;
    esac
}

reject_unsafe_path() {
    local label="$1" value="$2"
    [ -n "$value" ] || die "$label is empty"
    case "$value" in
        *$'\n'*|*$'\r'*|*$'\t'*) die "$label contains control characters" ;;
    esac
    [ "$value" != "/" ] || die "$label must not be the filesystem root"
    [ "$value" != "$HOME" ] || die "$label must not be the home directory itself"
}

ensure_dir() {
    if ! mkdir -p -- "$1" 2>/dev/null; then
        die "cannot create $1 (check ownership and permissions)"
    fi
    if [ ! -w "$1" ]; then
        die "directory is not writable: $1"
    fi
}

file_size() {
    stat -c%s -- "$1" 2>/dev/null || stat -f%z -- "$1" 2>/dev/null || printf '0'
}

json_escape() {
    local value="$1"
    value="${value//\\/\\\\}"
    value="${value//\"/\\\"}"
    printf '%s' "$value"
}

desktop_quote() {
    local value="$1"
    value="${value//\\/\\\\}"
    value="${value//\"/\\\"}"
    value="${value//\`/\\\`}"
    value="${value//\$/\\\$}"
    value="${value//%/%%}"
    printf '"%s"' "$value"
}

# A model directory is only considered usable when it contains a real
# checkpoint. The repository ships 1-byte placeholder files under models/,
# and installing those would produce a silently broken application.
model_is_valid() {
    local dir="$1" file size
    [ -d "$dir" ] || return 1

    for file in "$dir/pytorch_model.bin" "$dir/model.safetensors"; do
        if [ -f "$file" ] && [ "$(file_size "$file")" -gt 1000000 ]; then
            return 0
        fi
    done

    if [ -f "$dir/pytorch_model.bin.index.json" ] ||
        [ -f "$dir/model.safetensors.index.json" ]; then
        for file in "$dir"/*.bin "$dir"/*.safetensors; do
            [ -f "$file" ] || continue
            if [ "$(file_size "$file")" -gt 1000000 ]; then
                return 0
            fi
        done
    fi

    for file in "$dir"/pytorch_model-*.bin "$dir"/*.safetensors; do
        [ -f "$file" ] || continue
        if [ "$(file_size "$file")" -gt 1000000 ]; then
            return 0
        fi
    done
    return 1
}

dir_has_entries() {
    [ -d "$1" ] && [ -n "$(find "$1" -mindepth 1 -maxdepth 1 -print -quit 2>/dev/null)" ]
}

manifest_value() {
    local key="$1"
    sed -n "s/^${key}=//p" "$MANIFEST" | head -n 1
}

# ---------------------------------------------------------------------------
# Options
# ---------------------------------------------------------------------------

FEATURES=""
DO_BUILD=1
DO_MODELS=1
DO_DESKTOP=1
DO_UNINSTALL=0
RESET_CONFIG=0
BINARY_PATH=""
ICON_SRC=""
MODELS_SRC="${FLUENT_MODELS_SRC:-$SCRIPT_DIR/models}"
APP_DIR_ARG=""
BIN_DIR_ARG=""
CONFIG_DIR_ARG=""
MODELS_DIR_ARG=""

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
        --binary)
            [ "$#" -ge 2 ] || die "--binary requires a file"
            BINARY_PATH="$2"
            shift 2
            ;;
        --binary=*)
            BINARY_PATH="${1#*=}"
            shift
            ;;
        --skip-models)
            DO_MODELS=0
            shift
            ;;
        --models-src)
            [ "$#" -ge 2 ] || die "--models-src requires a directory"
            MODELS_SRC="$2"
            shift 2
            ;;
        --models-src=*)
            MODELS_SRC="${1#*=}"
            shift
            ;;
        --icon)
            [ "$#" -ge 2 ] || die "--icon requires a file"
            ICON_SRC="$2"
            shift 2
            ;;
        --icon=*)
            ICON_SRC="${1#*=}"
            shift
            ;;
        --no-desktop)
            DO_DESKTOP=0
            shift
            ;;
        --reset-config)
            RESET_CONFIG=1
            shift
            ;;
        --uninstall)
            DO_UNINSTALL=1
            shift
            ;;
        --app-dir)
            [ "$#" -ge 2 ] || die "--app-dir requires a path"
            APP_DIR_ARG="$2"
            shift 2
            ;;
        --app-dir=*)
            APP_DIR_ARG="${1#*=}"
            shift
            ;;
        --install-dir)
            [ "$#" -ge 2 ] || die "--install-dir requires a path"
            APP_DIR_ARG="$2"
            shift 2
            ;;
        --install-dir=*)
            APP_DIR_ARG="${1#*=}"
            shift
            ;;
        --bin-dir)
            [ "$#" -ge 2 ] || die "--bin-dir requires a path"
            BIN_DIR_ARG="$2"
            shift 2
            ;;
        --bin-dir=*)
            BIN_DIR_ARG="${1#*=}"
            shift
            ;;
        --config-dir)
            [ "$#" -ge 2 ] || die "--config-dir requires a path"
            CONFIG_DIR_ARG="$2"
            shift 2
            ;;
        --config-dir=*)
            CONFIG_DIR_ARG="${1#*=}"
            shift
            ;;
        --models-dir)
            [ "$#" -ge 2 ] || die "--models-dir requires a path"
            MODELS_DIR_ARG="$2"
            shift 2
            ;;
        --models-dir=*)
            MODELS_DIR_ARG="${1#*=}"
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

# ---------------------------------------------------------------------------
# Platform layout
# ---------------------------------------------------------------------------

# XDG variables must be absolute; relative values are ignored, matching the
# `directories` crate used by the application.
xdg_dir() {
    local var="$1" fallback="$2" value
    value="${!var:-}"
    case "$value" in
        /*) printf '%s' "$value" ;;
        *) printf '%s' "$fallback" ;;
    esac
}

CONFIG_HOME="$(xdg_dir XDG_CONFIG_HOME "$HOME/.config")"
DATA_HOME="$(xdg_dir XDG_DATA_HOME "$HOME/.local/share")"
STATE_HOME="$(xdg_dir XDG_STATE_HOME "$HOME/.local/state")"

case "$(uname -s)" in
    Linux*)
        PLATFORM="Linux"
        DEFAULT_APP_DIR="$HOME/.local/lib/fluent"
        DEFAULT_BIN_DIR="$HOME/.local/bin"
        DEFAULT_CONFIG_DIR="$CONFIG_HOME/fluent"
        DEFAULT_MODELS_DIR="$DATA_HOME/fluent/models"
        DESKTOP_DIR="$DATA_HOME/applications"
        ICON_DIR="$DATA_HOME/icons/hicolor/scalable/apps"
        ;;
    Darwin*)
        PLATFORM="macOS"
        FLUENT_BASE="$HOME/Library/Application Support/io.fluent.Fluent"
        DEFAULT_APP_DIR="$FLUENT_BASE/app"
        DEFAULT_BIN_DIR="$HOME/.local/bin"
        DEFAULT_CONFIG_DIR="$FLUENT_BASE"
        DEFAULT_MODELS_DIR="$FLUENT_BASE/models"
        DESKTOP_DIR=""
        ICON_DIR=""
        DO_DESKTOP=0
        ;;
    MINGW*|MSYS*|CYGWIN*)
        die "Windows installation is not provided by this script yet. Build with 'cargo build --release -p fluent-gui' and see docs/WINDOWS.md."
        ;;
    *)
        die "unsupported operating system: $(uname -s)"
        ;;
esac

STATE_DIR="$STATE_HOME/fluent"
MANIFEST="$STATE_DIR/install-manifest"

APP_DIR="$(expand_tilde "${APP_DIR_ARG:-${FLUENT_APP_DIR:-${FLUENT_INSTALL_DIR:-$DEFAULT_APP_DIR}}}")"
BIN_DIR="$(expand_tilde "${BIN_DIR_ARG:-${FLUENT_BIN_DIR:-$DEFAULT_BIN_DIR}}")"
CONFIG_DIR="$(expand_tilde "${CONFIG_DIR_ARG:-${FLUENT_CONFIG_DIR:-$DEFAULT_CONFIG_DIR}}")"
MODELS_DIR="$(expand_tilde "${MODELS_DIR_ARG:-${FLUENT_MODELS_DIR:-$DEFAULT_MODELS_DIR}}")"
APP_DIR="$(absolute_path "$APP_DIR")"
BIN_DIR="$(absolute_path "$BIN_DIR")"
CONFIG_DIR="$(absolute_path "$CONFIG_DIR")"
MODELS_DIR="$(absolute_path "$MODELS_DIR")"

# ---------------------------------------------------------------------------
# Uninstall
# ---------------------------------------------------------------------------

if [ "$DO_UNINSTALL" -eq 1 ]; then
    if [ ! -f "$MANIFEST" ]; then
        die "no installation manifest at $MANIFEST; nothing to uninstall (or pass the same --app-dir used at install time and remove files manually)"
    fi

    APP_DIR="$(manifest_value app_dir)"
    BIN_DIR="$(manifest_value bin_dir)"
    CONFIG_DIR="$(manifest_value config_dir)"
    MODELS_DIR="$(manifest_value models_dir)"
    LAUNCHER="$(manifest_value launcher)"
    DESKTOP_FILE="$(manifest_value desktop)"
    ICON_FILE="$(manifest_value icon)"

    reject_unsafe_path "app directory in manifest" "$APP_DIR"

    echo "Removing the Fluent application (configuration and models are preserved)..."

    if [ -n "$LAUNCHER" ] && [ -L "$LAUNCHER" ]; then
        if [ "$(readlink "$LAUNCHER")" = "$APP_DIR/fluent" ]; then
            rm -f -- "$LAUNCHER"
        else
            warn "not removing $LAUNCHER: it does not point to this installation"
        fi
    elif [ -n "$LAUNCHER" ] && [ -e "$LAUNCHER" ]; then
        warn "not removing $LAUNCHER: it is not a symlink created by Fluent"
    fi

    if [ -n "$DESKTOP_FILE" ] && [ -f "$DESKTOP_FILE" ]; then
        if grep -q '^X-Fluent-Installer=1$' "$DESKTOP_FILE"; then
            rm -f -- "$DESKTOP_FILE"
        else
            warn "not removing $DESKTOP_FILE: it was not created by this installer"
        fi
    fi

    if [ -n "$ICON_FILE" ] && [ -f "$ICON_FILE" ]; then
        case "$ICON_FILE" in
            */icons/hicolor/*/apps/io.fluent.Fluent.svg|*/icons/hicolor/*/apps/io.fluent.Fluent.png)
                rm -f -- "$ICON_FILE"
                ;;
            *)
                warn "not removing $ICON_FILE: unexpected icon path"
                ;;
        esac
    fi

    rm -f -- "$APP_DIR/fluent" "$APP_DIR/LICENSE"
    rm -f -- "$MANIFEST"

    if [ -d "$APP_DIR" ] && ! rmdir -- "$APP_DIR" 2>/dev/null; then
        warn "left $APP_DIR in place because it contains other files"
    fi
    if [ -d "$STATE_DIR" ] && ! rmdir -- "$STATE_DIR" 2>/dev/null; then
        :
    fi

    if command -v update-desktop-database >/dev/null 2>&1 && [ -n "$DESKTOP_DIR" ]; then
        update-desktop-database "$DESKTOP_DIR" >/dev/null 2>&1 || true
    fi
    if command -v gtk-update-icon-cache >/dev/null 2>&1 && [ -n "$ICON_DIR" ]; then
        gtk-update-icon-cache -f -t "${ICON_DIR%/scalable/apps}" >/dev/null 2>&1 || true
    fi

    cat <<EOF

Fluent application files removed.

Preserved on purpose:
  Configuration : $CONFIG_DIR
  Models        : $MODELS_DIR

Delete those directories manually only if you no longer need your settings or
model files.
EOF
    exit 0
fi

# ---------------------------------------------------------------------------
# Install
# ---------------------------------------------------------------------------

if [ -n "$BINARY_PATH" ]; then
    DO_BUILD=0
fi

reject_unsafe_path "application directory" "$APP_DIR"
reject_unsafe_path "launcher directory" "$BIN_DIR"
reject_unsafe_path "configuration directory" "$CONFIG_DIR"
reject_unsafe_path "model directory" "$MODELS_DIR"

LAUNCHER="$BIN_DIR/fluent"
if [ -e "$LAUNCHER" ] || [ -L "$LAUNCHER" ]; then
    if [ -L "$LAUNCHER" ] && [ "$(readlink "$LAUNCHER")" = "$APP_DIR/fluent" ]; then
        :
    else
        die "$LAUNCHER already exists and is not Fluent's launcher. Move it away or choose --bin-dir."
    fi
fi

if [ "$FEATURES" = "metal" ] && [ "$PLATFORM" != "macOS" ]; then
    die "--metal is only supported on macOS"
fi
if [ "$FEATURES" = "cuda" ] && [ "$PLATFORM" = "macOS" ]; then
    die "--cuda is not supported on macOS; use --metal instead"
fi

if [ "$DO_BUILD" -eq 1 ]; then
    command -v cargo >/dev/null 2>&1 || die \
        "cargo was not found. Install Rust from https://rustup.rs and try again."
    echo "Building Fluent for $PLATFORM..."
    if [ -n "$FEATURES" ]; then
        (cd "$SCRIPT_DIR" && cargo build --release --locked -p fluent-gui --features "$FEATURES")
    else
        (cd "$SCRIPT_DIR" && cargo build --release --locked -p fluent-gui)
    fi
fi

if [ -n "$BINARY_PATH" ]; then
    BIN_SRC="$(absolute_path "$(expand_tilde "$BINARY_PATH")")"
else
    BIN_SRC="$SCRIPT_DIR/target/release/fluent"
fi
[ -f "$BIN_SRC" ] || die "binary not found: $BIN_SRC (run without --skip-build or pass --binary FILE)"

if [ -n "$ICON_SRC" ]; then
    if [ "$PLATFORM" = "macOS" ]; then
        warn "icon integration is not implemented on macOS yet; --icon will be ignored"
    fi
    ICON_SRC="$(absolute_path "$(expand_tilde "$ICON_SRC")")"
    [ -f "$ICON_SRC" ] || die "icon not found: $ICON_SRC"
    case "$ICON_SRC" in
        *.svg) ;;
        *) die "only SVG icons are supported; convert the asset to SVG first" ;;
    esac
fi

ensure_dir "$APP_DIR"
ensure_dir "$BIN_DIR"
ensure_dir "$CONFIG_DIR"
ensure_dir "$MODELS_DIR"
if [ "$DO_DESKTOP" -eq 1 ]; then
    ensure_dir "$DESKTOP_DIR"
fi

echo "Installing Fluent into $APP_DIR..."
TMP_BIN="$(mktemp "$APP_DIR/.fluent.XXXXXX")"
if command -v install >/dev/null 2>&1; then
    install -m 0755 "$BIN_SRC" "$TMP_BIN"
else
    cp -- "$BIN_SRC" "$TMP_BIN"
    chmod 0755 "$TMP_BIN"
fi
mv -f -- "$TMP_BIN" "$APP_DIR/fluent"

if [ -f "$SCRIPT_DIR/LICENSE" ]; then
    install -m 0644 "$SCRIPT_DIR/LICENSE" "$APP_DIR/LICENSE" 2>/dev/null ||
        cp -- "$SCRIPT_DIR/LICENSE" "$APP_DIR/LICENSE"
fi

ln -sfn "$APP_DIR/fluent" "$LAUNCHER"

# -- Configuration ----------------------------------------------------------
CONFIG_FILE="$CONFIG_DIR/config.json"
if [ -f "$CONFIG_FILE" ] && [ "$RESET_CONFIG" -eq 0 ]; then
    echo "Configuration preserved: $CONFIG_FILE (use --reset-config to regenerate)"
else
    TMP_CONFIG="$(mktemp "$CONFIG_DIR/.config.XXXXXX")"
    FAST_JSON="$(json_escape "$MODELS_DIR/nllb-200-1.3b")"
    ACCURATE_JSON="$(json_escape "$MODELS_DIR/nllb-200-3.3b")"
    cat > "$TMP_CONFIG" <<EOF
{
  "default_source_language": "auto",
  "default_target_language": "eng_Latn",
  "default_mode": "fast",
  "theme": "system",
  "models": {
    "fast": "$FAST_JSON",
    "accurate": "$ACCURATE_JSON"
  },
  "generation": {
    "max_length": 512,
    "num_beams": 4
  }
}
EOF
    chmod 0644 "$TMP_CONFIG"
    mv -f -- "$TMP_CONFIG" "$CONFIG_FILE"
    echo "Configuration written: $CONFIG_FILE"
fi

# -- Models -----------------------------------------------------------------
install_model() {
    local name="$1"
    local target="$MODELS_DIR/$name"
    local source="" candidate staging

    ensure_dir "$target"

    if model_is_valid "$target"; then
        echo "  $name: already installed (left untouched)"
        return 0
    fi
    if dir_has_entries "$target"; then
        warn "$name: $target contains files but no complete checkpoint; leaving it untouched"
        return 0
    fi

    if [ "$DO_MODELS" -eq 0 ]; then
        echo "  $name: not installed (--skip-models); place files in $target"
        return 0
    fi

    for candidate in \
        "${MODELS_SRC:+$MODELS_SRC/$name}" \
        "$HOME/.fluent/models/$name"; do
        [ -n "$candidate" ] || continue
        if model_is_valid "$candidate"; then
            source="$candidate"
            break
        fi
        if dir_has_entries "$candidate"; then
            warn "$name: $candidate has no complete checkpoint (placeholder files?)"
        fi
    done

    if [ -z "$source" ]; then
        echo "  $name: no complete checkpoint found; place the model files in $target"
        return 0
    fi

    echo "  $name: installing from $source"
    staging="$MODELS_DIR/.fluent-$name-$$"
    if [ -e "$staging" ]; then
        die "temporary path already exists: $staging"
    fi
    mkdir -- "$staging"
    if cp -al -- "$source/." "$staging/" 2>/dev/null; then
        echo "    hard-linked (no extra disk space used)"
    else
        find "$staging" -mindepth 1 -delete 2>/dev/null || true
        if ! cp -a -- "$source/." "$staging/"; then
            find "$staging" -mindepth 1 -delete 2>/dev/null || true
            rmdir -- "$staging" 2>/dev/null || true
            die "could not copy model files from $source"
        fi
        echo "    copied (hard links unavailable)"
    fi
    if ! rmdir -- "$target" 2>/dev/null; then
        find "$staging" -mindepth 1 -delete 2>/dev/null || true
        rmdir -- "$staging" 2>/dev/null || true
        die "$target became non-empty during installation"
    fi
    mv -- "$staging" "$target"
}

if [ "$DO_MODELS" -eq 1 ]; then
    if [ -n "$MODELS_SRC" ]; then
        MODELS_SRC="$(absolute_path "$(expand_tilde "$MODELS_SRC")")"
        [ -d "$MODELS_SRC" ] || warn "model source directory not found: $MODELS_SRC"
    fi
    install_model "nllb-200-1.3b"
    install_model "nllb-200-3.3b"
else
    ensure_dir "$MODELS_DIR/nllb-200-1.3b"
    ensure_dir "$MODELS_DIR/nllb-200-3.3b"
    echo "Model installation skipped (--skip-models)."
fi

# -- Desktop integration ----------------------------------------------------
DESKTOP_FILE=""
ICON_FILE=""
if [ "$DO_DESKTOP" -eq 1 ]; then
    if [ -n "$ICON_SRC" ]; then
        ensure_dir "$ICON_DIR"
        ICON_FILE="$ICON_DIR/$APP_ID.svg"
        install -m 0644 "$ICON_SRC" "$ICON_FILE" 2>/dev/null ||
            cp -- "$ICON_SRC" "$ICON_FILE"
    fi

    DESKTOP_FILE="$DESKTOP_DIR/$APP_ID.desktop"
    TMP_DESKTOP="$(mktemp "$DESKTOP_DIR/.$APP_ID.XXXXXX")"
    EXEC_LINE="$(desktop_quote "$LAUNCHER")"
    {
        printf '%s\n' '[Desktop Entry]'
        printf '%s\n' 'Type=Application'
        printf '%s\n' 'Name=Fluent'
        printf '%s\n' 'GenericName=Translation'
        printf '%s\n' 'Comment=Local-first, offline translation with NLLB-200'
        printf 'Exec=%s\n' "$EXEC_LINE"
        if [ -n "$ICON_FILE" ]; then
            printf 'Icon=%s\n' "$APP_ID"
        fi
        printf '%s\n' 'Terminal=false'
        printf '%s\n' 'Categories=Utility;'
        printf '%s\n' 'Keywords=translation;language;offline;'
        printf '%s\n' 'StartupWMClass=io.fluent.Fluent'
        printf '%s\n' 'X-Fluent-Installer=1'
    } > "$TMP_DESKTOP"
    chmod 0644 "$TMP_DESKTOP"
    mv -f -- "$TMP_DESKTOP" "$DESKTOP_FILE"

    if command -v desktop-file-validate >/dev/null 2>&1; then
        if ! desktop-file-validate "$DESKTOP_FILE" 2>/dev/null; then
            warn "desktop-file-validate reported an issue with $DESKTOP_FILE"
        fi
    fi
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database "$DESKTOP_DIR" >/dev/null 2>&1 || true
    fi
    if [ -n "$ICON_FILE" ] && command -v gtk-update-icon-cache >/dev/null 2>&1; then
        gtk-update-icon-cache -f -t "${ICON_DIR%/scalable/apps}" >/dev/null 2>&1 || true
    fi
fi

# -- Manifest ---------------------------------------------------------------
ensure_dir "$STATE_DIR"
TMP_MANIFEST="$(mktemp "$STATE_DIR/.manifest.XXXXXX")"
{
    printf 'app_dir=%s\n' "$APP_DIR"
    printf 'bin_dir=%s\n' "$BIN_DIR"
    printf 'config_dir=%s\n' "$CONFIG_DIR"
    printf 'models_dir=%s\n' "$MODELS_DIR"
    printf 'launcher=%s\n' "$LAUNCHER"
    printf 'desktop=%s\n' "$DESKTOP_FILE"
    printf 'icon=%s\n' "$ICON_FILE"
} > "$TMP_MANIFEST"
chmod 0644 "$TMP_MANIFEST"
mv -f -- "$TMP_MANIFEST" "$MANIFEST"

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------

cat <<EOF

Fluent installed successfully.

  Platform     : $PLATFORM
  Application  : $APP_DIR/fluent
  Launcher     : $LAUNCHER
  Configuration: $CONFIG_FILE
  Models       : $MODELS_DIR
EOF
if [ -n "$DESKTOP_FILE" ]; then
    printf '  Desktop entry: %s\n' "$DESKTOP_FILE"
fi
cat <<EOF

Run it with:

  "$LAUNCHER"

If "$BIN_DIR" is not on your PATH, add it to your shell configuration.
Uninstall with:

  ./install.sh --uninstall

Configuration and model files are preserved by uninstall.
EOF
