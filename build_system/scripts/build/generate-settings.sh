#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
LOG="$ROOT/cache/containers/logs/build.log"

mkdir -p "$ROOT/cache/target/config/profiles"
mkdir -p "$(dirname "$LOG")"

dump_build_log() {
  status=$?
  if [ "$status" -ne 0 ]; then
    echo "build_system/scripts/build/generate-settings.sh failed with exit code $status" >&2
    if [ -f "$LOG" ]; then
      echo "---- cache/containers/logs/build.log tail ----" >&2
      tail -200 "$LOG" >&2 || true
      echo "---- end cache/containers/logs/build.log tail ----" >&2
    fi
  fi
  exit "$status"
}
trap dump_build_log EXIT

echo "[generate] $(date +%H:%M:%S) exporting MCP tool defs" >> "$LOG"
(cd "$ROOT" && cargo run -p capsem-core --bin mcp_export 2>>"$LOG" > cache/target/config/profiles/catalog.generated.json)
echo "[generate] $(date +%H:%M:%S) generating schema + defaults + mock" >> "$LOG"
# `$1`, when given, is where the two tracked settings files go. The checker
# passes a scratch directory so the gate never writes into its own checked-in
# source; without it they land in the checkout as before.
if [ -n "${1:-}" ]; then
  (cd "$ROOT" && uv run --project build_system --frozen python build_system/scripts/build/generate_schema.py --settings-dir "$1" >> "$LOG" 2>&1)
else
  (cd "$ROOT" && uv run --project build_system --frozen python build_system/scripts/build/generate_schema.py >> "$LOG" 2>&1)
fi
