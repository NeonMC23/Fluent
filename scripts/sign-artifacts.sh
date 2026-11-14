#!/usr/bin/env bash
#
# Create detached Minisign signatures for final distribution archives.
# The private key is never stored in the repository or printed.
#
# Usage:
#   MINISIGN_SECRET_KEY_FILE=/secure/minisign.key ./scripts/sign-artifacts.sh dist/*.tar.gz dist/*.zip
#   MINISIGN_SECRET_KEY="<key file contents>" ./scripts/sign-artifacts.sh ...
#
# The CI secret must contain the contents of the Minisign secret key file.
# Prefer an unencrypted key only inside the protected CI secret, or add a
# non-interactive password mechanism supported by your Minisign version.

set -euo pipefail

KEY_FILE="${MINISIGN_SECRET_KEY_FILE:-}"
if [ -z "$KEY_FILE" ] && [ -n "${MINISIGN_SECRET_KEY:-}" ]; then
    KEY_FILE="$(mktemp "${TMPDIR:-/tmp}/fluent-minisign.XXXXXX")"
    chmod 0600 "$KEY_FILE"
    trap 'rm -f -- "$KEY_FILE"' EXIT
    printf '%s' "$MINISIGN_SECRET_KEY" > "$KEY_FILE"
fi
[ -n "$KEY_FILE" ] || {
    echo "error: set MINISIGN_SECRET_KEY_FILE or MINISIGN_SECRET_KEY" >&2
    exit 2
}
[ -f "$KEY_FILE" ] || { echo "error: key not found: $KEY_FILE" >&2; exit 2; }
if command -v minisign >/dev/null 2>&1; then
    SIGNER="minisign"
elif command -v rsign >/dev/null 2>&1; then
    SIGNER="rsign"
else
    echo "error: neither minisign nor rsign is installed" >&2
    exit 2
fi
[ "$#" -gt 0 ] || { echo "usage: $0 <archive>..." >&2; exit 2; }

for file in "$@"; do
    [ -f "$file" ] || { echo "error: missing artifact: $file" >&2; exit 1; }
    rm -f -- "$file.minisig"
    if [ "$SIGNER" = "minisign" ]; then
        minisign -S -s "$KEY_FILE" -m "$file" -x "$file.minisig"
    else
        rsign sign -s "$KEY_FILE" -x "$file.minisig" "$file"
    fi
done
echo "Signed $# artifact(s) with $SIGNER."
