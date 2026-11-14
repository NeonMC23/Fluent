#!/usr/bin/env bash
#
# Verify detached Minisign signatures for downloaded artifacts.
#
# Usage:
#   MINISIGN_PUBLIC_KEY_FILE=/secure/minisign.pub ./scripts/verify-artifacts.sh dist/*.tar.gz dist/*.zip

set -euo pipefail

PUBLIC_KEY="${MINISIGN_PUBLIC_KEY_FILE:-}"
if [ -z "$PUBLIC_KEY" ] && [ -n "${MINISIGN_PUBLIC_KEY:-}" ]; then
    PUBLIC_KEY="$(mktemp "${TMPDIR:-/tmp}/fluent-minisign-pub.XXXXXX")"
    chmod 0600 "$PUBLIC_KEY"
    trap 'rm -f -- "$PUBLIC_KEY"' EXIT
    printf '%s' "$MINISIGN_PUBLIC_KEY" > "$PUBLIC_KEY"
fi
[ -n "$PUBLIC_KEY" ] || {
    echo "error: set MINISIGN_PUBLIC_KEY_FILE or MINISIGN_PUBLIC_KEY" >&2
    exit 2
}
[ -f "$PUBLIC_KEY" ] || { echo "error: public key not found: $PUBLIC_KEY" >&2; exit 2; }
if command -v minisign >/dev/null 2>&1; then
    VERIFIER="minisign"
elif command -v rsign >/dev/null 2>&1; then
    VERIFIER="rsign"
else
    echo "error: neither minisign nor rsign is installed" >&2
    exit 2
fi
[ "$#" -gt 0 ] || { echo "usage: $0 <archive>..." >&2; exit 2; }

for file in "$@"; do
    [ -f "$file" ] || { echo "error: missing artifact: $file" >&2; exit 1; }
    [ -f "$file.minisig" ] || { echo "error: missing signature: $file.minisig" >&2; exit 1; }
    if [ "$VERIFIER" = "minisign" ]; then
        minisign -V -p "$PUBLIC_KEY" -m "$file" -x "$file.minisig"
    else
        rsign verify -p "$PUBLIC_KEY" -x "$file.minisig" "$file"
    fi
done
echo "Verified $# artifact(s) with $VERIFIER."
