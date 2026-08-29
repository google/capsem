#!/usr/bin/env bash
# Assemble the asset-channel staging payload and prove its public shape locally.
set -euo pipefail

if [ "$#" -ne 5 ]; then
    echo "usage: $0 <channel> <manifest-version> <fixture-dir> <dist-dir> <evidence-dir>" >&2
    exit 2
fi

CHANNEL=$1
MANIFEST_VERSION=$2
FIXTURE_DIR=$3
DIST_DIR=$4
EVIDENCE_DIR=$5
SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
REPO_ROOT=$(cd -- "$SCRIPT_DIR/../../.." && pwd)
cd "$REPO_ROOT"

absolute_path() {
    case "$1" in
        /*) printf '%s\n' "$1" ;;
        *) printf '%s/%s\n' "$REPO_ROOT" "$1" ;;
    esac
}

FIXTURE_DIR=$(absolute_path "$FIXTURE_DIR")
DIST_DIR=$(absolute_path "$DIST_DIR")
EVIDENCE_DIR=$(absolute_path "$EVIDENCE_DIR")
for path in "$FIXTURE_DIR" "$DIST_DIR" "$EVIDENCE_DIR"; do
    if [ -e "$path" ]; then
        echo "refusing stale asset staging path: $path" >&2
        exit 1
    fi
done

uv run --project build_system --frozen python scripts/write-release-site-ci-fixture.py "$FIXTURE_DIR"
cargo run -p capsem-admin -- assets channel build \
    --manifest "file://$FIXTURE_DIR/assets/manifest.json" \
    --assets-dir "$FIXTURE_DIR/assets" \
    --channel "$CHANNEL" \
    --manifest-version "$MANIFEST_VERSION" \
    --out-dir "$DIST_DIR"
CAPSEM_RELEASE_GRAPH="$DIST_DIR" \
    CAPSEM_RELEASE_CHANNEL_DIST="$DIST_DIR" \
    bash scripts/check-web-surface.sh release-site-build
cargo run -p capsem-admin -- assets channel check \
    --channel "$CHANNEL" \
    --dist "$DIST_DIR"
mkdir -p "$EVIDENCE_DIR"
uv run --project build_system --frozen python scripts/check-release-site-contract.py \
    --base-url "file://$DIST_DIR" \
    --channel "$CHANNEL" \
    --dist "$DIST_DIR" \
    --attempts 1 \
    --delay-seconds 0 \
    --snapshot-out "$EVIDENCE_DIR/candidate-release.json"
