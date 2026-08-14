#!/bin/bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

require_release_site_astro() {
    if [[ ! -x "$ROOT/release-site/node_modules/.bin/astro" ]]; then
        echo "release-site Astro is missing; run the CI step 'Install release site dependencies' (cd release-site && pnpm install --frozen-lockfile)." >&2
        exit 1
    fi
}

surface="${1:-}"
case "$surface" in
    frontend-verify)
        # Type-check and unit tests, and deliberately not the build. Clippy
        # waits on the frontend because capsem-app embeds `frontend/dist` at
        # compile time via `tauri::generate_context!` -- it reads the build's
        # output and nothing else. While all three ran as one step, clippy
        # waited through these two as well, and through
        # `audit.generated-settings` on top: `mock-settings.generated` is
        # imported by three files, every one of them under `__tests__`, so it
        # is a dependency of this half alone. `frontend-build` needs no such
        # edge, and the build below is the same command it runs.
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
        require_release_site_astro
        # Two roles, two names. The generated distribution happens to be both
        # here -- Astro renders the graph it contains, and the overlay writes
        # the rendered site back into it -- which is exactly why one name
        # survived as long as it did. Other callers pass a graph *file*.
        : "${CAPSEM_RELEASE_GRAPH:?CAPSEM_RELEASE_GRAPH is required}"
        : "${CAPSEM_RELEASE_CHANNEL_DIST:?CAPSEM_RELEASE_CHANNEL_DIST is required}"
        pnpm --dir release-site run build:channel
        test -s "$CAPSEM_RELEASE_CHANNEL_DIST/404.html"
        grep -q "Artifact not found" "$CAPSEM_RELEASE_CHANNEL_DIST/404.html"
        ;;
    release-site)
        # The Astro surface only. The release-channel parity proof that used to
        # run here is `release-channel`: it spends its time in
        # `cargo run -p capsem-admin`, not in Astro, and holding the
        # `astro_build` claim across a Rust build stalled the one surface that
        # gates clippy.
        require_release_site_astro
        pnpm --dir release-site run check
        pnpm --dir release-site run test:coverage
        ;;
    release-channel)
        require_release_site_astro
        work="$ROOT/target/web-parity"
        fixture="$work/release-site-fixture"
        dist="$work/release-channel"
        graph_sources="$work/release-graphs"
        graph_dist="$work/release-channel-from-graphs"
        rm -rf "$work"
        mkdir -p "$work"

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
        echo "usage: $0 {frontend|frontend-build|docs|site|release-site|release-channel|release-site-build}" >&2
        exit 2
        ;;
esac
