#!/bin/bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

surface="${1:-}"
case "$surface" in
    frontend)
        pnpm --dir frontend run check
        if [[ -n "${CAPSEM_FRONTEND_JUNIT:-}" ]]; then
            (
                cd frontend
                npx vitest run --coverage \
                    --reporter=default \
                    --reporter=junit \
                    --outputFile="$CAPSEM_FRONTEND_JUNIT"
            )
        else
            pnpm --dir frontend run test
        fi
        pnpm --dir frontend run build
        ;;
    frontend-build)
        pnpm --dir frontend run build
        ;;
    docs)
        pnpm --dir docs run build
        ;;
    site)
        pnpm --dir site run build
        ;;
    release-site-build)
        : "${CAPSEM_RELEASE_CHANNEL_DIST:?CAPSEM_RELEASE_CHANNEL_DIST is required}"
        pnpm --dir release-site run build:channel
        test -s "$CAPSEM_RELEASE_CHANNEL_DIST/404.html"
        grep -q "Artifact not found" "$CAPSEM_RELEASE_CHANNEL_DIST/404.html"
        ;;
    release-site)
        work="$ROOT/target/web-parity"
        fixture="$work/release-site-fixture"
        dist="$work/release-channel"
        graph_sources="$work/release-graphs"
        graph_dist="$work/release-channel-from-graphs"
        rm -rf "$work"
        mkdir -p "$work"

        pnpm --dir release-site run check
        pnpm --dir release-site run test:coverage
        uv run python scripts/write-release-site-ci-fixture.py "$fixture"
        uv run python scripts/build-complete-release-channel.py \
            --channel-source "stable=file://$fixture/assets/manifest.json" \
            --primary-channel stable \
            --assets-dir "$fixture/assets" \
            --asset-source-base \
                "https://github.com/google/capsem/releases/download/assets-v{asset_version}" \
            --manifest-version 1.0.2 \
            --out-dir "$dist" \
            --release-site "file://$fixture" \
            --allow-mirror-missing

        # Production binary publication consumes the already-published graph
        # manifests, not the legacy asset manifest above. Exercise that exact
        # preservation/materialization path locally as part of the same gate.
        mkdir -p "$graph_sources"
        cp "$dist/assets/stable/manifest.json" "$graph_sources/stable.json"
        cp "$dist/assets/nightly/manifest.json" "$graph_sources/nightly.json"
        uv run python scripts/build-complete-release-channel.py \
            --channel-source "stable=file://$graph_sources/stable.json" \
            --channel-source "nightly=file://$graph_sources/nightly.json" \
            --primary-channel stable \
            --assets-dir "$fixture/assets" \
            --asset-source-base \
                "https://github.com/google/capsem/releases/download/assets-v{asset_version}" \
            --manifest-version 1.0.2 \
            --profile-source-root "$ROOT" \
            --out-dir "$graph_dist"
        ;;
    *)
        echo "usage: $0 {frontend|frontend-build|docs|site|release-site|release-site-build}" >&2
        exit 2
        ;;
esac
