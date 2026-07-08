#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LOG="$ROOT/target/build.log"

mkdir -p "$ROOT/target"
mkdir -p "$ROOT/target/config/profiles"

dump_build_log() {
  status=$?
  if [ "$status" -ne 0 ]; then
    echo "scripts/generate-settings.sh failed with exit code $status" >&2
    if [ -f "$LOG" ]; then
      echo "---- target/build.log tail ----" >&2
      tail -200 "$LOG" >&2 || true
      echo "---- end target/build.log tail ----" >&2
    fi
  fi
  exit "$status"
}
trap dump_build_log EXIT

echo "[generate] $(date +%H:%M:%S) exporting MCP tool defs" >> "$LOG"
(cd "$ROOT" && cargo run -p capsem-core --bin mcp_export 2>>"$LOG" > target/config/profiles/catalog.generated.json)
echo "[generate] $(date +%H:%M:%S) generating schema + defaults + mock" >> "$LOG"
(cd "$ROOT" && uv run python scripts/generate_schema.py >> "$LOG" 2>&1)
