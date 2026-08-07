#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "${BASH_SOURCE[0]%/*}" && pwd)"
exec python3 "$SCRIPT_DIR/check-hardcoded-release-selections.py"
