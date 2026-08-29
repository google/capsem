#!/bin/bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$ROOT"
source "$ROOT/build_system/scripts/build/lib/exec_lock.sh"

require_release_site_astro() {
    if [[ ! -x "$ROOT/build_system/release_site/node_modules/.bin/astro" ]]; then
        echo "release-site Astro is missing; run the CI step 'Install release site dependencies' (cd build_system/release_site && pnpm install --frozen-lockfile)." >&2
        exit 1
    fi
}

# The one lock every release-site and frontend build takes, whoever starts it.
#
# The gate declares `astro_build` in `[execution.exclusives]`, which is a
# `threading.Lock`: it orders steps inside one gate process and coordinates
# nothing with a pytest run, a second gate, or a developer running this script
# by hand. The tests take a file lock instead. Two mechanisms, one Astro
# staging directory derived from the project root, and no way for either to see
# the other.
#
# So the lock lives here, at the place that actually runs the build, and is
# derived from the repository rather than from the caller's environment. Every
# entry point serializes because they all come through this file.
BUILD_LOCK="$ROOT/target/capsem-release-site-build.lock"

astro_build() {
    run_with_exec_lock "$BUILD_LOCK" "$@"
}

surface="${1:-}"
case "$surface" in
    frontend-verify)
        # Type-check and unit tests, and deliberately not the build. Clippy
        # waits on the frontend because capsem-app embeds `web/app/dist` at
        # compile time via `tauri::generate_context!` -- it reads the build's
        # output and nothing else. While all three ran as one step, clippy
        # waited through these two as well, and through
        # `audit.generated-settings` on top: `mock-settings.generated` is
        # imported by three files, every one of them under `__tests__`, so it
        # is a dependency of this half alone. `frontend-build` needs no such
        # edge, and the build below is the same command it runs.
        pnpm --dir web/app run check
        if [[ -n "${CAPSEM_FRONTEND_JUNIT:-}" ]]; then
            (
                cd web/app
                npx vitest run --coverage \
                    --reporter=default \
                    --reporter=junit \
                    --outputFile="$CAPSEM_FRONTEND_JUNIT"
            )
        else
            pnpm --dir web/app run test
        fi
        ;;
    frontend-build)
        astro_build pnpm --dir web/app run build
        ;;
    docs)
        astro_build pnpm --dir web/docs run build
        ;;
    site)
        pnpm --dir web/marketing run check
        astro_build pnpm --dir web/marketing run build
        ;;
    release-site-build)
        require_release_site_astro
        # Two roles, two names. The generated distribution happens to be both
        # here -- Astro renders the graph it contains, and the overlay writes
        # the rendered site back into it -- which is exactly why one name
        # survived as long as it did. Other callers pass a graph *file*.
        : "${CAPSEM_RELEASE_GRAPH:?CAPSEM_RELEASE_GRAPH is required}"
        : "${CAPSEM_RELEASE_CHANNEL_DIST:?CAPSEM_RELEASE_CHANNEL_DIST is required}"
        astro_build pnpm --dir build_system/release_site run build:channel
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
        pnpm --dir build_system/release_site run check
        pnpm --dir build_system/release_site run test:coverage
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

        uv run --project build_system --frozen python build_system/release_site/scripts/write-release-site-ci-fixture.py "$fixture"
        uv run --project build_system --frozen python build_system/scripts/release/build-complete-release-channel.py \
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
        uv run --project build_system --frozen python build_system/scripts/release/build-complete-release-channel.py \
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
