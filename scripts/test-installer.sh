#!/usr/bin/env bash
#
# Disposable installer test. Uses only temporary HOME directories, a fake
# executable and a fake 2 MB checkpoint. Never touches the real user profile
# and never builds or loads a model.

set -euo pipefail

ROOT="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/fluent-installer-test.XXXXXX")"
cleanup() {
    rm -rf -- "$TMP"
}
trap cleanup EXIT

fail() {
    printf 'FAIL: %s\n' "$*" >&2
    exit 1
}

assert_file() {
    [ -f "$1" ] || fail "expected file: $1"
}

assert_symlink_to() {
    [ -L "$1" ] || fail "expected symlink: $1"
    [ "$(readlink "$1")" = "$2" ] || fail "$1 does not point to $2"
}

assert_absent() {
    [ ! -e "$1" ] && [ ! -L "$1" ] || fail "expected absent: $1"
}

run_in_home() {
    local home="$1"
    shift
    env HOME="$home" \
        XDG_CONFIG_HOME="$home/.config" \
        XDG_DATA_HOME="$home/.local/share" \
        XDG_STATE_HOME="$home/.local/state" \
        "$@"
}

# Fake install payload.
FAKE_BIN="$TMP/fake fluent"
printf '#!/bin/sh\nexit 0\n' > "$FAKE_BIN"
chmod 0755 "$FAKE_BIN"

FAKE_SRC="$TMP/model source"
mkdir -p "$FAKE_SRC/nllb-200-1.3b"
truncate -s 2000000 "$FAKE_SRC/nllb-200-1.3b/pytorch_model.bin"
printf '{"model_type":"m2m_100"}\n' > "$FAKE_SRC/nllb-200-1.3b/config.json"
printf '{"version":"1.0","model":{}}\n' > "$FAKE_SRC/nllb-200-1.3b/tokenizer.json"

ICON="$TMP/icon.svg"
printf '<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"></svg>\n' > "$ICON"

HOME_DIR="$TMP/home with space"
mkdir -p "$HOME_DIR"

APP="$HOME_DIR/.local/lib/fluent/fluent"
LAUNCHER="$HOME_DIR/.local/bin/fluent"
CONFIG="$HOME_DIR/.config/fluent/config.json"
MODEL="$HOME_DIR/.local/share/fluent/models/nllb-200-1.3b"
DESKTOP="$HOME_DIR/.local/share/applications/io.fluent.Fluent.desktop"
ICON_DST="$HOME_DIR/.local/share/icons/hicolor/scalable/apps/io.fluent.Fluent.svg"

echo '1. fresh install'
run_in_home "$HOME_DIR" "$ROOT/install.sh" \
    --binary "$FAKE_BIN" --models-src "$FAKE_SRC" --icon "$ICON" > "$TMP/install.log" 2>&1
assert_file "$APP"
assert_symlink_to "$LAUNCHER" "$APP"
assert_file "$CONFIG"
assert_file "$MODEL/pytorch_model.bin"
assert_file "$DESKTOP"
assert_file "$ICON_DST"
python3 -m json.tool "$CONFIG" > /dev/null || fail 'config.json is not valid JSON'
grep -q "^Exec=\"$LAUNCHER\"" "$DESKTOP" || fail 'desktop Exec path is wrong'
if command -v desktop-file-validate >/dev/null 2>&1; then
    desktop-file-validate "$DESKTOP" > /dev/null || fail 'desktop entry validation failed'
fi

echo '2. repeat install preserves user data'
printf '{"user_marker":true}\n' > "$CONFIG"
printf 'keep me\n' > "$MODEL/user-note.txt"
printf '#!/bin/sh\necho updated\n' > "$TMP/fake fluent.2"
chmod 0755 "$TMP/fake fluent.2"
run_in_home "$HOME_DIR" "$ROOT/install.sh" \
    --binary "$TMP/fake fluent.2" --models-src "$FAKE_SRC" --icon "$ICON" > "$TMP/reinstall.log" 2>&1
grep -q 'user_marker' "$CONFIG" || fail 'config was overwritten on reinstall'
assert_file "$MODEL/user-note.txt"
grep -q updated "$APP" || fail 'binary was not replaced on reinstall'

echo '3. uninstall preserves config and models'
run_in_home "$HOME_DIR" "$ROOT/uninstall.sh" > "$TMP/uninstall.log" 2>&1
assert_absent "$APP"
assert_absent "$LAUNCHER"
assert_absent "$DESKTOP"
assert_absent "$ICON_DST"
assert_file "$CONFIG"
assert_file "$MODEL/user-note.txt"
assert_file "$FAKE_SRC/nllb-200-1.3b/pytorch_model.bin"

echo '4. missing binary fails safely'
EMPTY_HOME="$TMP/home missing"; mkdir -p "$EMPTY_HOME"
if run_in_home "$EMPTY_HOME" "$ROOT/install.sh" --binary "$TMP/does-not-exist" > "$TMP/missing.log" 2>&1; then
    fail 'missing binary did not fail'
fi
assert_absent "$EMPTY_HOME/.local/lib/fluent/fluent"

echo '5. --no-desktop and --skip-models'
PLAIN_HOME="$TMP/home plain"; mkdir -p "$PLAIN_HOME"
run_in_home "$PLAIN_HOME" "$ROOT/install.sh" \
    --binary "$FAKE_BIN" --skip-models --no-desktop > "$TMP/plain.log" 2>&1
assert_absent "$PLAIN_HOME/.local/share/applications/io.fluent.Fluent.desktop"
[ -d "$PLAIN_HOME/.local/share/fluent/models/nllb-200-1.3b" ] ||
    fail 'model directory was not created'

echo '6. placeholder models are rejected'
PLACEHOLDER_HOME="$TMP/home placeholder"; mkdir -p "$PLACEHOLDER_HOME"
run_in_home "$PLACEHOLDER_HOME" "$ROOT/install.sh" \
    --binary "$FAKE_BIN" --models-src "$ROOT/models" --no-desktop > "$TMP/placeholder.log" 2>&1
assert_absent "$PLACEHOLDER_HOME/.local/share/fluent/models/nllb-200-1.3b/pytorch_model.bin"

echo 'PASS: installer tests completed'
