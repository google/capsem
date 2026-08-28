#!/usr/bin/env bash
set -euo pipefail

ROOT="${CAPSEM_REPO_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
AUDIT_REQUIREMENTS="$ROOT/target/python-audit-requirements.txt"

mkdir -p "$(dirname "$AUDIT_REQUIREMENTS")"
cd "$ROOT"
uv export \
    --project build_system \
    --frozen \
    --quiet \
    --format requirements-txt \
    --locked \
    --no-emit-project \
    --output-file "$AUDIT_REQUIREMENTS" \
    >/dev/null
uv run --project build_system --frozen pip-audit \
    -s osv \
    --requirement "$AUDIT_REQUIREMENTS" \
    --require-hashes \
    --disable-pip
