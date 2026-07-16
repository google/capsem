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
        ;;
    release-site)
        work="$ROOT/target/web-parity"
        fixture="$work/release-site-fixture"
        dist="$work/release-channel"
        rm -rf "$work"
        mkdir -p "$work"

        pnpm --dir release-site run check
        pnpm --dir release-site run test:coverage
        uv run python scripts/write-release-site-ci-fixture.py "$fixture"
        cargo run -p capsem-admin -- assets channel build \
            --manifest "file://$fixture/assets/manifest.json" \
            --assets-dir "$fixture/assets" \
            --asset-source-base \
                "https://github.com/google/capsem/releases/download/assets-v{asset_version}" \
            --channel stable \
            --manifest-version 1.0.2 \
            --out-dir "$dist"
        CAPSEM_RELEASE_CHANNEL_DIST="$dist" \
            bash scripts/check-web-surface.sh release-site-build
        cargo run -p capsem-admin -- assets channel check \
            --channel stable \
            --dist "$dist"
        ;;
    *)
        echo "usage: $0 {frontend|frontend-build|docs|site|release-site|release-site-build}" >&2
        exit 2
        ;;
esac
