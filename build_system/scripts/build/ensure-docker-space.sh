#!/usr/bin/env bash
# Compatibility entrypoint: the Python controller owns all policy and cleanup.
set -euo pipefail

# `CDPATH= cd` clears CDPATH for this one command so `cd --` cannot land
# somewhere else entirely. ShellCheck reads the empty assignment as a typo.
# shellcheck disable=SC1007
SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
exec uv run --no-project --python 3.12 python \
    "$SCRIPT_DIR/docker-storage-policy.py" enforce \
    --rail "${1:-default}" \
    --label "${2:-preflight}"
