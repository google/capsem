#!/usr/bin/env bash
# Check the manual and its primary destinations after CDN propagation.
set -euo pipefail
: "${SITE_URL:?the deployed docs origin is required}"
readonly ATTEMPTS="${SMOKE_ATTEMPTS:-6}"
readonly PAUSE_SECONDS="${SMOKE_PAUSE_SECONDS:-10}"
WORK=$(mktemp)
trap 'rm -f "$WORK"' EXIT
check_page() {
    curl --max-time 20 -fsSL "$SITE_URL/$1" -o "$WORK" && grep -q "$2" "$WORK"
}
for attempt in $(seq 1 "$ATTEMPTS"); do
    echo "Checking $SITE_URL (attempt $attempt of $ATTEMPTS)..."
    if check_page "" "How Capsem works" &&
       check_page "sdk/" "Getting started with the SDK" &&
       check_page "architecture/security/" "Security architecture" &&
       check_page "getting-started/installation/" "Installation"; then
        echo "docs.capsem.org smoke passed."
        exit 0
    fi
    sleep "$PAUSE_SECONDS"
done
echo "docs.capsem.org smoke failed after deploy." >&2
exit 1
