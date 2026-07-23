#!/usr/bin/env bash
# Compatibility entrypoint: the Python controller owns all policy and cleanup.
set -euo pipefail

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
exec uv run python "$SCRIPT_DIR/docker-storage-policy.py" enforce \
    --rail "${1:-default}" \
    --label "${2:-preflight}"
