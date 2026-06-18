#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LOG="$ROOT/target/build.log"

mkdir -p "$ROOT/target"
mkdir -p "$ROOT/target/config/profiles"
echo "[generate] $(date +%H:%M:%S) exporting MCP tool defs" >> "$LOG"
(cd "$ROOT" && cargo run --bin mcp_export 2>>"$LOG" > target/config/profiles/catalog.generated.json)
echo "[generate] $(date +%H:%M:%S) generating schema + defaults + mock" >> "$LOG"
(cd "$ROOT" && uv run python scripts/generate_schema.py >> "$LOG" 2>&1)
