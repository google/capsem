#!/bin/bash
# Fail unless a freshly built Capsem CLI embeds the exact source revision.
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
BINARY="${1:?usage: $0 BINARY [EXPECTED_GIT_REVISION]}"
EXPECTED_REVISION="${2:-}"

if [ -z "$EXPECTED_REVISION" ]; then
    EXPECTED_REVISION=$(git -C "$ROOT" rev-parse --short HEAD)
fi
if [ ! -x "$BINARY" ]; then
    echo "ERROR: build provenance check requires an executable Capsem binary: $BINARY" >&2
    exit 1
fi

VERSION_OUTPUT=$("$BINARY" version)
EXPECTED_MARKER="(build $EXPECTED_REVISION."
case "$VERSION_OUTPUT" in
    *"$EXPECTED_MARKER"*)
        echo "Exact build provenance verified: $EXPECTED_REVISION"
        ;;
    *)
        echo "ERROR: Capsem binary does not embed exact source revision $EXPECTED_REVISION" >&2
        echo "version output: $VERSION_OUTPUT" >&2
        exit 1
        ;;
esac
