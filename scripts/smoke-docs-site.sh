#!/usr/bin/env bash
# Prove the deployed docs site serves the current content, and that the retired
# route stays retired.
#
# Lifted out of docs.yaml:build, where it was twenty-three executable lines of
# YAML holding a retry loop and thirteen chained conditions -- the exact shape
# the shell-body boundary exists to stop. Inline it was reachable by no linter
# and callable by no test; here ShellCheck reads it like any other script and
# the two checks below can be exercised against a local server.
#
# The checks and their order are unchanged. What was `&&`-chained into one
# `if` is now two functions, because a thirteen-term conjunction reports
# failure as "something in there was false" -- and after a deploy the useful
# question is *which* one.
set -euo pipefail

: "${SITE_URL:?the deployed docs origin is required}"
: "${OLD_ROUTE_URL:?the retired route to prove gone is required}"

readonly ATTEMPTS="${SMOKE_ATTEMPTS:-6}"
readonly PAUSE_SECONDS="${SMOKE_PAUSE_SECONDS:-10}"
readonly WORK="${TMPDIR:-/tmp}"

# The site is live: HTML, and this release's content rather than a stale build.
serves_current_docs() {
    curl -fsSLI "$SITE_URL/" -o "$WORK/docs-headers.txt" &&
        grep -qi '^content-type: text/html' "$WORK/docs-headers.txt" &&
        curl -fsSL "$SITE_URL/" -o "$WORK/docs-index.html" &&
        grep -qi '<main[ >]' "$WORK/docs-index.html" &&
        grep -q 'Capsem 0.6 documentation' "$WORK/docs-index.html" &&
        grep -q 'pre-release qualification' "$WORK/docs-index.html"
}

# The retired route is gone and stays gone: it must not be cached, must say so,
# and must carry none of the old page back with it.
retired_route_is_gone() {
    curl -fsSL -D "$WORK/docs-old-headers.txt" "$OLD_ROUTE_URL" -o "$WORK/docs-old-route.html" &&
        grep -qi '^cache-control:.*no-store' "$WORK/docs-old-headers.txt" &&
        grep -q 'Documentation route unavailable' "$WORK/docs-old-route.html" &&
        grep -q 'pre-release qualification' "$WORK/docs-old-route.html" &&
        ! grep -qi 'Getting Started' "$WORK/docs-old-route.html" &&
        ! grep -qi 'install.sh' "$WORK/docs-old-route.html" &&
        ! grep -qi 'starlight' "$WORK/docs-old-route.html"
}

for attempt in $(seq 1 "$ATTEMPTS"); do
    echo "Checking $SITE_URL (attempt $attempt of $ATTEMPTS)..."
    if serves_current_docs && retired_route_is_gone; then
        echo "docs.capsem.org smoke passed."
        exit 0
    fi
    # Retried rather than failed on the first miss: a CDN deploy propagates,
    # so an early attempt legitimately sees the previous build.
    sleep "$PAUSE_SECONDS"
done

echo "docs.capsem.org smoke failed after deploy." >&2
exit 1
