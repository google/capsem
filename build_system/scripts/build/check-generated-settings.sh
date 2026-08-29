#!/usr/bin/env bash
set -euo pipefail

SCRIPT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
ROOT="${1:-$SCRIPT_ROOT}"
FRESH="$(mktemp -d)"
trap 'rm -rf "$FRESH"' EXIT

# These files are checked in and must exactly match the generator. Generated
# into a scratch directory and compared, rather than overwritten in place and
# diffed against a snapshot: the old shape rewrote the gate's own checked-in
# source on every run. Byte-identical output made that invisible, and it made
# the gate unable to run against a source tree it may not write to.
TRACKED_FILES=(
  config/settings/schema.generated.json
  config/settings/ui-metadata.generated.json
)

for file in "${TRACKED_FILES[@]}"; do
  if [ ! -f "$ROOT/$file" ]; then
    echo "ERROR: tracked generated settings file is missing: $file" >&2
    exit 1
  fi
done

bash "$ROOT/build_system/scripts/build/generate-settings.sh" "$FRESH"

failed=0
# The mock is gitignored and the web checks import it, so it is still written
# into the checkout; the tracked pair is not.
if [ ! -f "$ROOT/web/app/src/lib/mock-settings.generated.ts" ]; then
  echo "ERROR: settings generator did not create: web/app/src/lib/mock-settings.generated.ts" >&2
  failed=1
fi

for file in "${TRACKED_FILES[@]}"; do
  fresh="$FRESH/$(basename "$file")"
  if [ ! -f "$fresh" ]; then
    echo "ERROR: settings generator did not create: $file" >&2
    failed=1
    continue
  fi
  if ! cmp -s "$ROOT/$file" "$fresh"; then
    echo "ERROR: generated settings drifted: $file" >&2
    diff -u "$ROOT/$file" "$fresh" || true
    failed=1
  fi
done

if [ "$failed" -ne 0 ]; then
  echo "Generated tracked files were refreshed or required outputs are missing." >&2
  echo "Review the generator output, then rerun just test." >&2
  exit 1
fi
