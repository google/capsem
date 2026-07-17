#!/usr/bin/env bash
set -euo pipefail

SCRIPT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ROOT="${1:-$SCRIPT_ROOT}"
SNAPSHOT="$(mktemp -d)"
trap 'rm -rf "$SNAPSHOT"' EXIT

# These files are checked in and must exactly match the generator. Snapshot
# them before generation so the same gate works in both dirty developer trees
# and pristine CI checkouts.
TRACKED_FILES=(
  config/settings/schema.generated.json
  config/settings/ui-metadata.generated.json
)

# The frontend mock is intentionally gitignored, so a clean checkout cannot
# snapshot it. It must instead be created by the generator before later web
# gates run. Keep the complete output list explicit so a silent generator
# regression still fails here.
GENERATED_FILES=(
  "${TRACKED_FILES[@]}"
  frontend/src/lib/mock-settings.generated.ts
)

for file in "${TRACKED_FILES[@]}"; do
  if [ ! -f "$ROOT/$file" ]; then
    echo "ERROR: tracked generated settings file is missing: $file" >&2
    exit 1
  fi
  mkdir -p "$SNAPSHOT/$(dirname "$file")"
  cp "$ROOT/$file" "$SNAPSHOT/$file"
done

bash "$ROOT/scripts/generate-settings.sh"

failed=0
for file in "${GENERATED_FILES[@]}"; do
  if [ ! -f "$ROOT/$file" ]; then
    echo "ERROR: settings generator did not create: $file" >&2
    failed=1
  fi
done

for file in "${TRACKED_FILES[@]}"; do
  if ! cmp -s "$SNAPSHOT/$file" "$ROOT/$file"; then
    echo "ERROR: generated settings drifted: $file" >&2
    diff -u "$SNAPSHOT/$file" "$ROOT/$file" || true
    failed=1
  fi
done

if [ "$failed" -ne 0 ]; then
  echo "Generated tracked files were refreshed or required outputs are missing." >&2
  echo "Review the generator output, then rerun just test." >&2
  exit 1
fi
