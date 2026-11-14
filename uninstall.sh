#!/usr/bin/env bash
#
# Thin wrapper around `install.sh --uninstall`. The installer keeps a manifest
# under the XDG state directory, so configuration and model assets are always
# preserved.

set -euo pipefail
SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
exec "$SCRIPT_DIR/install.sh" --uninstall "$@"
