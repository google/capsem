# Capsem Justfile
#
# The public surface is intentionally small and locked by
# config/public-surface.toml. New public recipes require explicit approval.
#
#   dev [ui|frontend|tui]  development surfaces
#   build                  desktop application + embedded frontend
#   build-all              all host binaries, desktop app, and docs
#   build-docs             documentation and marketing sites
#   shell / exec           temporary VM interaction
#   run-service            idempotent local daemon
#   logs [sandbox|failure] service, VM, or failure evidence
#   doctor                 host, Docker/Colima, Tart, and asset readiness
#   smoke                  focused local integration feedback
#   test                   complete local all-artifact proof
#   release-binaries       publish packages for one channel
#   release-profile        publish one channel/profile
#
# Underscore recipes are implementation detail. CI may call a private primitive
# only when it is part of the canonical `test` graph.

binary := "target/debug/capsem"
cli_binary := "target/debug/capsem"
service_binary := "target/debug/capsem-service"
process_binary := "target/debug/capsem-process"
mcp_binary := "target/debug/capsem-mcp"
gateway_binary := "target/debug/capsem-gateway"
admin_binary := "target/debug/capsem-admin"
host_binaries := "target/debug/capsem target/debug/capsem-service target/debug/capsem-process target/debug/capsem-mcp target/debug/capsem-mcp-aggregator target/debug/capsem-mcp-builtin target/debug/capsem-gateway target/debug/capsem-tray target/debug/capsem-admin target/debug/capsem-tui target/debug/capsem-mock-server target/debug/capsem-bench-rs"
assets_dir := "assets"
entitlements := "entitlements.plist"
host_crates := "-p capsem-service -p capsem-process -p capsem -p capsem-tui -p capsem-mcp -p capsem-mcp-aggregator -p capsem-mcp-builtin -p capsem-gateway -p capsem-tray -p capsem-admin -p capsem-mock-server -p capsem-bench"
# Propagate Cargo.toml's version across the release cohort (capsem.gate.versions).
_stamp-version:
    @uv run capsem-gate stamp-version

# Build, test, and publish only Capsem binaries/packages for one channel.
release-binaries channel:
    #!/bin/bash
    set -euo pipefail
    # Fail in seconds on a dirty tree or the wrong branch. The authoritative
    # check still runs after the gate, since the state can drift during it.
    python3 scripts/publish-tested-main.py --precheck
    python3 scripts/extract-release-notes.py --check
    mkdir -p target/release-preflight
    RELEASE_GITHUB_TOKEN="${GITHUB_TOKEN:-$(gh auth token)}"
    RELEASE_REPOSITORY="${GITHUB_REPOSITORY:-google/capsem}"
    GITHUB_TOKEN="$RELEASE_GITHUB_TOKEN" \
        python3 scripts/fetch-channel-source-manifest.py \
            --channel "{{channel}}" \
            --repository "$RELEASE_REPOSITORY" \
            --require-profile-membership \
            --output target/release-preflight/channel-source.json
    TESTED_HEAD=$(git rev-parse HEAD)
    just test
    python3 scripts/publish-tested-main.py --expected-head "$TESTED_HEAD"
    python3 scripts/release-binaries.py "{{channel}}"

# Build, test, and publish exactly one channel/profile through capsem-admin.
release-profile channel profile:
    #!/bin/bash
    set -euo pipefail
    # Fail in seconds on a dirty tree or the wrong branch. The authoritative
    # check still runs after the gate, since the state can drift during it.
    python3 scripts/publish-tested-main.py --precheck
    TESTED_HEAD=$(git rev-parse HEAD)
    just test
    python3 scripts/publish-tested-main.py --expected-head "$TESTED_HEAD"
    cargo run -p capsem-admin -- release --channel "{{channel}}" --profile "{{profile}}"

# Compile all host binaries
_build-host:
    cargo build {{host_crates}}

# Run the terminal control UI against the installed gateway, or with
# `--fixture --snapshot` for deterministic render inspection.
_dev-tui *ARGS:
    cargo run -p capsem-tui -- {{ARGS}}

# Codesign all host binaries (macOS only, needed for Virtualization.framework)
_sign: _build-host
    #!/bin/bash
    if [[ "$(uname -s)" == "Darwin" ]]; then
        for bin in {{host_binaries}}; do
            codesign --sign - --entitlements {{entitlements}} --force "$bin"
        done
    fi

# Ensure capsem-service daemon is running with the current binary.
# Kills any existing dev-owned instance (via pidfile -- never pkill-by-name)
# and relaunches fresh. Honors CAPSEM_HOME / CAPSEM_RUN_DIR env vars so
# `just test` and `just smoke` run against an isolated test home
# without ever touching the user's locally installed capsem.
_ensure-service: _sign
    #!/bin/bash
    set -euo pipefail
    ROOT="{{justfile_directory()}}"
    arch=$(uname -m)
    [[ "$arch" == "arm64" ]] || arch="x86_64"
    GENERATED_PROFILES="$ROOT/target/config/profiles"
    if [ ! -d "$GENERATED_PROFILES" ]; then
        echo "ERROR: generated profiles missing at $GENERATED_PROFILES"
        echo "       Run just _materialize-config or a recipe that depends on it."
        exit 1
    fi
    # Resolve capsem home + run dir from env, matching the Rust helpers.
    CAPSEM_HOME_DIR="${CAPSEM_HOME:-$HOME/.capsem}"
    RUN_DIR="${CAPSEM_RUN_DIR:-$CAPSEM_HOME_DIR/run}"
    mkdir -p "$RUN_DIR"
    PIDFILE="$RUN_DIR/service.pid"
    SOCKET="$RUN_DIR/service.sock"
    # Kill ONLY the service this pidfile tracks -- no pkill by name.
    # Killing by pattern would take down a user's locally installed capsem
    # (or a parallel test run with a different CAPSEM_HOME).
    if [ -f "$PIDFILE" ]; then
        OLD_PID=$(cat "$PIDFILE" 2>/dev/null || true)
        if [ -n "$OLD_PID" ] && kill -0 "$OLD_PID" 2>/dev/null; then
            # SIGTERM the service; it propagates to child capsem-process VMs.
            kill "$OLD_PID" 2>/dev/null || true
            for _ in 1 2 3 4 5 6; do
                kill -0 "$OLD_PID" 2>/dev/null || break
                sleep 0.25
            done
            # Force-kill if still alive.
            if kill -0 "$OLD_PID" 2>/dev/null; then
                pgrep -P "$OLD_PID" | xargs -r kill -9 2>/dev/null || true
                kill -9 "$OLD_PID" 2>/dev/null || true
            fi
        fi
    fi
    rm -f "$PIDFILE" "$SOCKET"
    # Keep the dev service on the same installed-style profile/assets rail as
    # packages. Symlinking ~/.capsem/assets to a worktree can mix stale profile
    # pins with fresh assets and make profiles look broken in the UI.
    retired_user_config="user"".toml"
    rm -f "$CAPSEM_HOME_DIR/$retired_user_config" "$CAPSEM_HOME_DIR/service.toml"
    echo "event=retired_config_removed"
    ASSETS_DIR="$CAPSEM_HOME_DIR/assets"
    bash "$ROOT/scripts/sync-dev-assets.sh" "{{assets_dir}}" "$ASSETS_DIR"
    PROFILES_DIR="$CAPSEM_HOME_DIR/profiles"
    rm -rf "$PROFILES_DIR"
    mkdir -p "$PROFILES_DIR"
    cp -R "$GENERATED_PROFILES/." "$PROFILES_DIR/"
    echo "event=dev_profile_assets_materialized assets=$ASSETS_DIR profiles=$PROFILES_DIR"
    echo "Starting capsem-service (CAPSEM_HOME=$CAPSEM_HOME_DIR)..."
    # Close fd 3 on the service; otherwise the backgrounded service inherits
    # the execution-lock fd from `just smoke` / `just test` and keeps the
    # flock held after the outer shell exits, blocking subsequent runs.
    nohup env CAPSEM_PROFILES_DIR="$GENERATED_PROFILES" RUST_LOG=capsem=debug {{service_binary}} \
        --assets-dir "$ASSETS_DIR" \
        --process-binary {{process_binary}} \
        --foreground 3>&- >/dev/null 2>&1 &
    SVC_PID=$!
    echo "$SVC_PID" > "$PIDFILE"
    for i in $(seq 1 30); do
        if [ -S "$SOCKET" ] && curl -s --unix-socket "$SOCKET" --max-time 2 http://localhost/list >/dev/null 2>&1; then
            echo "capsem-service running (PID $SVC_PID)"
            exit 0
        fi
        sleep 0.5
    done
    echo "ERROR: capsem-service did not start within 15s"
    kill $SVC_PID 2>/dev/null
    rm -f "$PIDFILE"
    exit 1

# Start service daemon + Tauri GUI with hot-reloading
_dev-ui: _ensure-dev-ready _pnpm-install run-service
    #!/bin/bash
    set -euo pipefail
    source {{justfile_directory()}}/scripts/lib/exec_lock.sh
    acquire_exec_lock "$HOME/.capsem/run/execution.lock"
    CAPSEM_ASSETS_DIR={{assets_dir}} cargo tauri dev --config crates/capsem-app/tauri.conf.json

# Frontend-only dev server with mock data (no Tauri/VM needed)
_dev-frontend: _pnpm-install _generate-settings
    cd frontend && pnpm run dev

# Build the Tauri desktop app (capsem-app) with a fresh frontend bundle.
# IMPORTANT: the Tauri binary embeds frontend/dist at cargo compile time via
# tauri::generate_context!(), so rebuilding only the frontend has no effect
# on the running binary. This recipe keeps the two in lockstep.
#   just build          # debug binary at ./target/debug/capsem-app
#   just build release  # release binary at ./target/release/capsem-app
_build-ui profile="debug": _pnpm-install _generate-settings
    #!/bin/bash
    set -euo pipefail
    echo "=== Frontend build ==="
    bash scripts/check-web-surface.sh frontend-build
    echo ""
    echo "=== capsem-app ({{profile}}) build ==="
    if [[ "{{profile}}" == "release" ]]; then
        cargo build -p capsem-app --release
        echo ""
        echo "Built ./target/release/capsem-app"
    else
        cargo build -p capsem-app
        echo ""
        echo "Built ./target/debug/capsem-app"
    fi

# Frontend release gate used by Sprinty and docs.
# Build both public documentation surfaces.
build-docs: _pnpm-install
    bash scripts/check-web-surface.sh docs
    bash scripts/check-web-surface.sh site

# Select one deliberate development surface.
dev surface="ui" *ARGS:
    #!/bin/bash
    set -euo pipefail
    case "{{surface}}" in
        ui) just _dev-ui ;;
        frontend) just _dev-frontend ;;
        tui) just _dev-tui {{ARGS}} ;;
        *)
            echo "ERROR: dev surface must be ui, frontend, or tui" >&2
            exit 2
            ;;
    esac

# Build the desktop application with its embedded frontend.
build profile="debug":
    just _build-ui "{{profile}}"

# Build every host binary plus the desktop and documentation surfaces.
# VM/release assets remain profile-owned and are built by the canonical test
# and release workflows, not hidden inside a routine source build.
build-all profile="debug":
    just build "{{profile}}"
    just _build-host
    just build-docs

# Start service daemon + boot temporary VM + shell (~10s after first build)
shell: _prepared-runtime _ensure-service
    #!/bin/bash
    set -euo pipefail
    source {{justfile_directory()}}/scripts/lib/exec_lock.sh
    acquire_exec_lock "$HOME/.capsem/run/execution.lock"
    {{cli_binary}} shell

# Start capsem-service daemon (builds, signs, launches or reuses running instance)
run-service: _prepared-runtime _ensure-service

# Execute a command in a fresh temporary VM (auto-provisioned and destroyed)
# Usage: just exec "echo hello"   or   just exec "ls -la"
exec +CMD: run-service
    #!/bin/bash
    set -euo pipefail
    source {{justfile_directory()}}/scripts/lib/exec_lock.sh
    acquire_exec_lock "$HOME/.capsem/run/execution.lock"
    {{cli_binary}} run "{{CMD}}"


# Build kernel only for one profile/arch (CI-facing primitive).
_build-kernel arch profile="" output=assets_dir:
    #!/bin/bash
    set -euo pipefail
    PROFILE_ARG="{{profile}}"
    OUTPUT_ARG="{{output}}"
    if [[ -z "$PROFILE_ARG" ]]; then
        echo "ERROR: internal _build-kernel requires <arch> <profile-id>"
        exit 2
    fi
    just _install-tools
    CAPSEM_SKIP_ASSET_CHECK=1 CAPSEM_SKIP_KVM_CHECK=1 just doctor
    just _build-image-template "{{arch}}" "$PROFILE_ARG" "$OUTPUT_ARG" kernel
    just _docker-gc

# Build rootfs only for one profile/arch (CI-facing primitive).
_build-rootfs arch profile="" output=assets_dir:
    #!/bin/bash
    set -euo pipefail
    PROFILE_ARG="{{profile}}"
    OUTPUT_ARG="{{output}}"
    if [[ -z "$PROFILE_ARG" ]]; then
        echo "ERROR: internal _build-rootfs requires <arch> <profile-id>"
        exit 2
    fi
    just _install-tools
    CAPSEM_SKIP_ASSET_CHECK=1 CAPSEM_SKIP_KVM_CHECK=1 just doctor
    just _build-image-template "{{arch}}" "$PROFILE_ARG" "$OUTPUT_ARG" rootfs
    just _docker-gc

# Already-preflighted image-build primitive shared by public CI recipes and
# the canonical all-profile matrix. Public recipes own tool/doctor setup;
# test-assets owns that setup once through its _bootstrap dependencies.
_build-image-template arch profile output template:
    #!/bin/bash
    set -euo pipefail
    PROFILE_ARG="{{profile}}"
    OUTPUT_ARG="{{output}}"
    TEMPLATE_ARG="{{template}}"
    case "$TEMPLATE_ARG" in
        kernel|rootfs) ;;
        *)
            echo "ERROR: unsupported image template: $TEMPLATE_ARG" >&2
            exit 2
            ;;
    esac
    cargo run -p capsem-admin -- image build \
        --profile "config/profiles/${PROFILE_ARG}/profile.toml" \
        --config-root config \
        --output "$OUTPUT_ARG" \
        --arch "{{arch}}" \
        --template "$TEMPLATE_ARG" \
        --clean

# VM asset rebuild (kernel + rootfs). Profile is mandatory. Optional second arg
# restricts to one arch.
_build-assets profile="" arch="" output=assets_dir:
    #!/bin/bash
    set -euo pipefail
    PROFILE_ARG="{{profile}}"
    ARCH_ARG="{{arch}}"
    OUTPUT_ARG="{{output}}"
    if [[ -z "$PROFILE_ARG" ]]; then
        echo "ERROR: internal _build-assets requires <profile-id> [arm64|x86_64]"
        exit 2
    fi
    just _install-tools
    just _clean-stale
    CAPSEM_SKIP_ASSET_CHECK=1 CAPSEM_SKIP_KVM_CHECK=1 just doctor
    ARGS=(
        --profile "config/profiles/${PROFILE_ARG}/profile.toml"
        --config-root config
        --output "$OUTPUT_ARG"
        --clean
    )
    if [[ -n "$ARCH_ARG" ]]; then
        ARGS+=(--arch "$ARCH_ARG")
    fi
    cargo run -p capsem-admin -- image build "${ARGS[@]}"
    just _docker-gc

# Ironbank VM asset gate. This is the superset owner for the image-build work
# performed by release-assets.yaml: every checked-in profile, both published
# architectures, the exact CI-facing build primitives, generated-manifest
# validation, and a real shell marker from each profile-owned host-arch image.
# Outputs stay under target/ so the gate never mutates the developer's assets/.
_gate-assets: _bootstrap _install-tools _generate-settings _sign
    @uv run capsem-gate assets

# Run ALL tests: Rust + frontend + Python + injection + integration + bench + cross-compile + install e2e. No shortcuts.
#
# Runs against an isolated CAPSEM_HOME under target/test-home/ so the suite
# never kills or mutates the user's locally installed capsem. The flock is
# still honored for multi-agent coordination but now lives inside the test
# home, not the shared ~/.capsem/run.
_bootstrap:
    sh {{justfile_directory()}}/bootstrap.sh -y

# Bind the complete gate to the exact source state present at invocation. A
# developer may test deliberate uncommitted work; the gate must return every
# tracked and untracked non-ignored source byte unchanged.
test:
    @uv run capsem-gate candidate

# After the source-only fast gate passes, local composition constructs every
# artifact family before running the remaining modules used by release CI.
_test-candidate:
    just _bootstrap
    just _bound-docker-test-storage
    just _install-tools
    just _clean-stale
    just _check-generated-settings
    # Clear stale VM performance recordings once per gate run, so the modules
    # below accumulate one complete set instead of overwriting each other.
    rm -rf "{{justfile_directory()}}/target/test-benchmarks"
    just _prepared-runtime
    just _test-static
    just _test-artifacts
    just _test-functional
    just _test-glowup
    just _test-recipes

# Parser errors, source contracts, dependency vulnerabilities, lint, Clippy,
# and every JavaScript/web check run before Colima, bootstrap, artifacts, or
# VMs. This is private composition, not a public release shortcut.
_test-fast:
    just _check-generated-settings
    just _test-release-contracts
    uv run capsem-gate test-fast

_test-static: _clean-stale _check-generated-settings
    just _bound-docker-test-storage
    uv run capsem-gate test-static

_test-artifacts:
    uv run capsem-gate test-artifacts

_test-functional: _generate-settings
    uv run capsem-gate test-functional

_test-glowup:
    uv run capsem-gate test-glowup

_test-release-contracts: _release-site-pnpm-install
    uv run capsem-gate test-release-contracts

# Require Docker headroom without discarding content-addressed compiler caches.
# Cargo validates cached artifacts against the current source inputs; bounded
# reuse speeds forward fixes without weakening the before/after tree invariant.

_test-recipes:
    uv run python -m pytest tests/capsem-recipes/ -v --tb=short -m recipe

# Build the capsem-host-builder Docker image (cached, only rebuilds changed layers).
# See docker/Dockerfile.host-builder for contents.
_build-host-image:
    #!/bin/bash
    set -euo pipefail
    echo "=== Building capsem-host-builder image ==="
    docker build \
        -t capsem-host-builder:latest \
        -f docker/Dockerfile.host-builder \
        docker/
    # On Linux CI the checkout's owner is not this image's user, so git rejects
    # /src as "dubious ownership" -- and crates/capsem/build.rs answers that by
    # embedding "unknown" instead of failing, which is how a binary with no
    # source identity reaches the provenance check. Forcing a foreign UID
    # reproduces it here: git compares st_uid to euid in userspace, so the
    # check works even on macOS bind mounts, which do not enforce write
    # permission and therefore cannot surface the rest of that family.
    ROOT="{{justfile_directory()}}"
    EXPECTED=$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo "")
    if [ -n "$EXPECTED" ]; then
        ACTUAL=$(docker run --rm -v "$ROOT:/src" -w /src --user 4242:4242 \
            capsem-host-builder:latest git rev-parse --short HEAD 2>/dev/null || echo "")
        if [ "$ACTUAL" != "$EXPECTED" ]; then
            echo "ERROR: capsem-host-builder cannot read /src as a non-owner user." >&2
            echo "       Linux package builds will embed an 'unknown' build hash." >&2
            echo "       Fix: keep 'git config --system --add safe.directory /src'" >&2
            echo "       in docker/Dockerfile.host-builder." >&2
            exit 1
        fi
        echo "  [pass] host-builder reads /src as a non-owner user ($ACTUAL)"
    fi

# Execute the portable Linux host-crate suite through one checked-in runner.
# Linux CI calls this recipe natively. Mac-local `just test` calls it through
# capsem-host-builder so cfg(target_os = "linux") tests are not CI-only.
_gate-linux-rust:
    #!/bin/bash
    set -euo pipefail
    ROOT="{{justfile_directory()}}"
    if [ "$(uname -s)" = "Linux" ]; then
        CAPSEM_LINUX_RUST_OUTPUT_DIR="$ROOT" bash "$ROOT/scripts/test-linux-rust.sh"
        exit 0
    fi

    [ "$(uname -s)" = "Darwin" ] || {
        echo "ERROR: Linux Rust parity supports native Linux or Docker on macOS" >&2
        exit 1
    }

    # Native Linux CI runs the shared script directly. Only a Mac host needs
    # the Linux builder image, so do not make Linux CI build an unused image.
    just _build-host-image
    [ -f "$ROOT/Cargo.lock" ] || cargo generate-lockfile
    OUTPUT_DIR="$ROOT/target/linux-rust-coverage"
    HOST_UID=$(id -u)
    HOST_GID=$(id -g)
    mkdir -p "$OUTPUT_DIR/nextest"
    # Match the non-root GitHub runner. Running this suite as container root
    # makes chmod-based permission regressions impossible to observe.
    docker run --rm \
        -v capsem-linux-rust-cargo-registry:/usr/local/cargo/registry \
        -v capsem-linux-rust-cargo-git:/usr/local/cargo/git \
        -v capsem-linux-rust-rustup:/usr/local/rustup \
        -v capsem-linux-rust-target:/cargo-target \
        capsem-host-builder:latest \
        sh -c "chown -R $HOST_UID:$HOST_GID /usr/local/cargo/registry /usr/local/cargo/git /usr/local/rustup /cargo-target"
    docker run --rm \
        --user "$HOST_UID:$HOST_GID" \
        -e HOME=/tmp/capsem-home \
        -e CAPSEM_SKIP_KVM_TESTS=1 \
        -e CAPSEM_LINUX_RUST_OUTPUT_DIR=/linux-rust-output \
        --tmpfs /tmp:rw,exec,mode=1777 \
        -v "$ROOT:/src:ro" \
        -v "$OUTPUT_DIR:/linux-rust-output" \
        -v "$OUTPUT_DIR/nextest:/src/target/nextest" \
        -v capsem-linux-rust-cargo-registry:/usr/local/cargo/registry \
        -v capsem-linux-rust-cargo-git:/usr/local/cargo/git \
        -v capsem-linux-rust-rustup:/usr/local/rustup \
        -v capsem-linux-rust-target:/cargo-target \
        -w /src \
        capsem-host-builder:latest \
        bash /src/scripts/test-linux-rust.sh
    docker run --rm \
        -v "$OUTPUT_DIR:/linux-rust-output" \
        alpine chown -R "$HOST_UID:$HOST_GID" /linux-rust-output

# Run the production release SBOM generator over the exact current-version
# packages built by the canonical gate. Mac runs cover one .pkg plus both .deb
# architectures; native Linux qualification covers both .deb architectures.
_gate-host-package-sbom:
    #!/bin/bash
    set -euo pipefail
    ROOT="{{justfile_directory()}}"
    VERSION=$(grep '^version' "$ROOT/Cargo.toml" | head -1 | sed 's/.*"\(.*\)".*/\1/')
    shopt -s nullglob
    DEBS=("$ROOT"/dist/*"$VERSION"*.deb)
    [ "${#DEBS[@]}" -eq 2 ] || {
        echo "ERROR: expected exactly two current-version Linux packages, found ${#DEBS[@]}" >&2
        printf '  %s\n' "${DEBS[@]}" >&2
        exit 1
    }
    ARTIFACTS=("${DEBS[@]}")
    if [ "$(uname -s)" = "Darwin" ]; then
        PKG="$ROOT/packages/Capsem-$VERSION.pkg"
        test -s "$PKG"
        ARTIFACTS+=("$PKG")
    fi
    OUTPUT="$ROOT/target/ironbank-host-sbom.spdx.json"
    python3 "$ROOT/scripts/generate-host-binary-sbom.py" \
        --output "$OUTPUT" \
        "${ARTIFACTS[@]}"
    python3 - "$OUTPUT" "${#ARTIFACTS[@]}" <<'PY'
    import json
    import pathlib
    import sys

    document = json.loads(pathlib.Path(sys.argv[1]).read_text())
    if document.get("spdxVersion") != "SPDX-2.3":
        raise SystemExit("host SBOM is not SPDX-2.3")
    if not document.get("files"):
        raise SystemExit("host SBOM contains no packaged executables")
    print(f"host SBOM validated: artifacts={sys.argv[2]} files={len(document['files'])}")
    PY

# Remove cross-compilation image and cached volumes.
_clean-host-image:
    @uv run capsem-gate storage clean --scope all

_release-completed-linux-rust-target:
    @uv run capsem-gate storage release completed-linux-rust-target

_release-completed-docker-rails:
    @uv run capsem-gate storage release completed-docker-rails

_release-completed-buildkit-graph:
    @uv run capsem-gate storage release completed-buildkit-graph

_release-completed-package-rails:
    @uv run capsem-gate storage release completed-package-arm64
    @uv run capsem-gate storage release completed-package-x86_64

_release-deferred-install-target:
    @uv run capsem-gate storage release deferred-install-target

# repack-deb.sh below reads the materialized profile catalog from target/config,
# so this recipe owns filling it rather than leaving each call site to remember.
# Release CI never enters here: it consumes an already-built package with its
# staged profile cohort, so nothing it pulled can be clobbered.
# Build the full Linux release in a container (agent + deb).
# Uses the private cached capsem-host-builder image.
# Supports arm64 and x86_64 via native cross-compilation (no QEMU).
#
# The image runs natively on the host arch and cross-compiles via
# Rust --target + multiarch system libs. Named volumes cache cargo
# registry and build artifacts between runs. CARGO_TARGET_DIR=/cargo-target
# inside the container isolates from host macOS target/ directory.
#
# CI vs local divergences (keep in sync when changing either):
#   - CI runs on bare ubuntu runners; this runs in capsem-host-builder via docker
#   - Tauri signing keys: CI from secrets, local from private/tauri/
#   - See: .github/workflows/release.yaml build-app-linux job
_cross-compile arch="": _clean-stale _check-assets _generate-settings _materialize-config
    @uv run capsem-gate cross-compile {{arch}}

# Generate settings schema/UI metadata and frontend mock data.
_generate-settings:
    #!/bin/bash
    set -euo pipefail
    bash scripts/generate-settings.sh

# Generate tracked settings outputs and fail if the generator changed them.
# This is the local equivalent of CI's generate-then-git-diff drift gate, but
# compares before/after content so an intentional already-generated worktree
# change can still be tested before it is committed.
_check-generated-settings:
    #!/bin/bash
    set -euo pipefail
    ROOT="{{justfile_directory()}}"
    bash "$ROOT/scripts/check-generated-settings.sh" "$ROOT"

# Focused developer feedback; never release qualification. It shares the exact
# fail-fast source gate with `test` and release CI, then runs a smaller VM loop.
smoke:
    #!/bin/bash
    set -euo pipefail
    just _test-fast
    just _prepared-runtime
    # Smoke runs against an isolated CAPSEM_HOME so it doesn't stomp on a
    # locally installed capsem daemon. _ensure-service is invoked below
    # (not as a just dep) so it inherits the exported env vars.
    export CAPSEM_HOME="{{justfile_directory()}}/target/test-home/.capsem"
    export CAPSEM_RUN_DIR="$CAPSEM_HOME/run"
    # Lockfile lives OUTSIDE $CAPSEM_HOME so it survives `rm -rf $CAPSEM_HOME`
    # below. Acquired BEFORE the wipe: if a second `just smoke` were to run
    # past this line, the first's fd would be pinned to an unlinked inode
    # and the second would flock a brand-new inode unchallenged.
    source {{justfile_directory()}}/scripts/lib/exec_lock.sh
    acquire_exec_lock "{{justfile_directory()}}/target/capsem-test-execution.lock"
    # Wipe stale state so assertions that read <capsem_home>/logs or
    # <capsem_home>/sessions don't trip on artifacts from a previous run
    # (e.g. a 0-entry capsem-app launch log left by a crashed Tauri shell).
    # Matches the `just test` preamble; smoke inherited the leak when
    # CAPSEM_HOME isolation was introduced.
    rm -rf "$CAPSEM_HOME"
    mkdir -p "$CAPSEM_RUN_DIR" "$CAPSEM_HOME/sessions" "$CAPSEM_HOME/logs"
    cleanup_test_capsem_home_service() {
        PIDFILE="$CAPSEM_RUN_DIR/service.pid"
        SOCKET="$CAPSEM_RUN_DIR/service.sock"
        if [ -f "$PIDFILE" ]; then
            OLD_PID=$(cat "$PIDFILE" 2>/dev/null || true)
            if [ -n "$OLD_PID" ] && kill -0 "$OLD_PID" 2>/dev/null; then
                kill "$OLD_PID" 2>/dev/null || true
                for _ in 1 2 3 4 5 6 7 8; do
                    kill -0 "$OLD_PID" 2>/dev/null || break
                    sleep 0.25
                done
                if kill -0 "$OLD_PID" 2>/dev/null; then
                    CHILDREN=$(pgrep -P "$OLD_PID" 2>/dev/null || true)
                    if [ -n "$CHILDREN" ]; then
                        kill -9 $CHILDREN 2>/dev/null || true
                    fi
                    kill -9 "$OLD_PID" 2>/dev/null || true
                fi
            fi
        fi
        rm -f "$PIDFILE" "$SOCKET"
    }
    trap cleanup_test_capsem_home_service EXIT
    just _ensure-service
    SMOKE_LOG="{{justfile_directory()}}/target/smoke.log"
    mkdir -p "$(dirname "$SMOKE_LOG")"
    exec > >(tee "$SMOKE_LOG") 2>&1
    SMOKE_START=$SECONDS
    step() { STEP_START=$SECONDS; echo "=== $1 ==="; }
    step_done() { echo "  -> $(( SECONDS - STEP_START ))s"; echo ""; }
    step "capsem-doctor (in-VM diagnostics)"
    {{cli_binary}} doctor
    step_done
    step "Injection test"
    uv run python scripts/injection_test.py --binary {{binary}} --assets {{assets_dir}}
    step_done
    step "Integration test"
    uv run python scripts/integration_test.py --binary {{binary}} --assets {{assets_dir}}
    step_done
    step "Python integration tests (MCP + service + CLI + gateway, parallel groups)"
    # Pre-sign binaries so parallel test groups don't race on codesign
    for b in {{service_binary}} {{process_binary}}; do
        codesign --sign - --entitlements {{entitlements}} --force "$b" 2>/dev/null || true
    done
    # service+cli is the longest group (~67s serial) -- the big lever.
    # -n 2 + --dist=loadfile cuts it to ~36s. loadfile keeps all tests in
    # a file on the same worker so module-scoped fixtures don't rebuild.
    # Suspend/resume is host-resource sensitive under Apple VZ. Keep those
    # files out of the parallel phase and run them serially after the other
    # service/gateway/MCP tests finish; otherwise unrelated VMs can make
    # resume fail before the guest signals ready.
    MCP_SERIAL="tests/capsem-mcp/test_state_transitions.py"
    SVC_SERIAL=(
        "tests/capsem-service/test_svc_resume_paths.py"
        "tests/capsem-service/test_svc_suspend_corruption.py"
        "tests/capsem-service/test_svc_loop_device_after_resume.py"
    )
    CAPSEM_TEST_RUN_ID=smoke-mcp uv run python -m pytest tests/capsem-mcp/ -v --tb=short -m "mcp" \
        --ignore="$MCP_SERIAL" &
    PID_MCP=$!
    CAPSEM_TEST_RUN_ID=smoke-service-cli uv run python -m pytest tests/capsem-service/ tests/capsem-cli/ \
        -v --tb=short -m "integration" -n 2 --dist=loadfile \
        --ignore="${SVC_SERIAL[0]}" \
        --ignore="${SVC_SERIAL[1]}" \
        --ignore="${SVC_SERIAL[2]}" &
    PID_SVC=$!
    CAPSEM_TEST_RUN_ID=smoke-gateway uv run python -m pytest tests/capsem-gateway/ -v --tb=short -m "gateway" &
    PID_GW=$!
    FAIL=0
    wait $PID_MCP || FAIL=1
    wait $PID_SVC || FAIL=1
    wait $PID_GW || FAIL=1
    [ $FAIL -eq 0 ] || { echo "Python tests failed"; exit 1; }
    CAPSEM_TEST_RUN_ID=smoke-mcp-serial uv run python -m pytest "$MCP_SERIAL" -v --tb=short -m "mcp"
    CAPSEM_TEST_RUN_ID=smoke-service-serial uv run python -m pytest "${SVC_SERIAL[@]}" -v --tb=short -m "integration"
    step_done
    echo "Smoke test passed in $(( SECONDS - SMOKE_START ))s"
    just _clean-stale

# Run install e2e tests in Docker (Linux + systemd).
# Builds the real .deb (Tauri + repack), installs with dpkg -i (exercises
# deb-postinst.sh), then runs the pytest suite against the installed layout.
_prove-linux-deb: _test-install-harness-preflight
    @uv run capsem-gate prove-deb

_test-install-harness-preflight:
    @uv run capsem-gate install-image

_gate-install:
    @uv run capsem-gate install

# Check dev tools and dependencies. Pass "fix" to auto-fix.
doctor fix="": _pnpm-install
    @uv run capsem-gate doctor
    @scripts/doctor-common.sh {{ if fix == "fix" { "--fix" } else { "" } }}

# View service logs, a sandbox's logs, or the latest preserved test failure.
# `just logs`, `just logs <sandbox-id>`, `just logs failure`.
logs target="":
    #!/bin/bash
    set -euo pipefail
    case "{{target}}" in
        "")
            tail -f "$HOME/.capsem/run/service.log"
            ;;
        failure)
            latest=$(find test-artifacts -mindepth 1 -maxdepth 1 -type d \
                -print 2>/dev/null | sort -r | head -1)
            if [ -z "$latest" ]; then
                echo "No preserved test failure." >&2
                exit 1
            fi
            echo "$latest"
            find "$latest" -maxdepth 3 -type f -print
            ;;
        *)
            {{cli_binary}} logs "{{target}}"
            ;;
    esac

# Remove stale rootfs copies, orphan UDS sockets, and trim bloated incremental caches.
# See scripts/clean_stale.py for implementation (tested: tests/capsem-cleanup-script/).
_clean-stale:
    @uv run python3 scripts/clean_stale.py

# Auto-prune Docker after builds: stopped containers, dangling images, build cache >7d.
# Keeps named volumes (cross-compile cargo caches) and recent build cache for fast rebuilds.
_docker-gc:
    @uv run capsem-gate storage gc

# Enforce release-rail headroom while preserving content-addressed Cargo,
# registry, rustup, and recent BuildKit caches that make forward fixes fast.
_bound-docker-test-storage:
    @uv run capsem-gate storage release candidate-boundary
    @uv run capsem-gate storage ensure-space default candidate-boundary

# Explicit deep cleanup for a human-requested cold rebuild. The canonical gate
# deliberately does not call this recipe.
_clean-docker-test-targets:
    @uv run capsem-gate storage clean --scope working --rail default

# --- Internal helpers (hidden from `just --list`) ---

# Run doctor automatically on first use (creates .dev-setup sentinel)
_ensure-dev-ready:
    #!/bin/bash
    if [ ! -f .dev-setup ]; then
        echo "First run detected -- running doctor..."
        echo ""
        just doctor
    fi

# Auto-install Rust targets, components, and cargo tools
_install-tools:
    #!/bin/bash
    set -euo pipefail
    # Musl targets for cross-compiling guest binaries
    if ! rustup target list --installed | grep -q aarch64-unknown-linux-musl; then
        echo "Installing aarch64-unknown-linux-musl target..."
        rustup target add aarch64-unknown-linux-musl
    fi
    if ! rustup target list --installed | grep -q x86_64-unknown-linux-musl; then
        echo "Installing x86_64-unknown-linux-musl target..."
        rustup target add x86_64-unknown-linux-musl
    fi
    # rust-lld linker (from llvm-tools component)
    if ! rustup component list --installed | grep -q llvm-tools; then
        echo "Installing llvm-tools (provides rust-lld)..."
        rustup component add llvm-tools
    fi
    # cargo-llvm-cov for coverage
    if ! command -v cargo-llvm-cov &>/dev/null; then
        echo "Installing cargo-llvm-cov..."
        cargo install cargo-llvm-cov
    fi
    # b3sum for BLAKE3 checksums
    if ! command -v b3sum &>/dev/null; then
        echo "Installing b3sum..."
        cargo install b3sum --locked
    fi
    # cargo-audit for vulnerability scanning
    if ! command -v cargo-audit &>/dev/null; then
        echo "Installing cargo-audit..."
        cargo install cargo-audit
    fi
    # Tauri CLI
    if ! cargo tauri --version &>/dev/null; then
        echo "Installing Tauri CLI..."
        cargo install tauri-cli
    fi
    # cargo-sbom for SPDX generation
    if ! command -v cargo-sbom &>/dev/null; then
        echo "Installing cargo-sbom..."
        cargo install cargo-sbom --locked
    fi

# Verify VM assets exist (vmlinuz, initrd.img, rootfs)
_check-assets:
    #!/bin/bash
    set -euo pipefail
    dir="{{assets_dir}}"
    # Map host architecture to asset directory name
    arch=$(uname -m | sed 's/aarch64/arm64/;s/arm64/arm64/')
    missing=()
    if [ -f "$dir/$arch/vmlinuz" ]; then
        # Per-arch layout: assets/{arch}/vmlinuz
        for f in vmlinuz initrd.img rootfs.erofs; do
            [ -f "$dir/$arch/$f" ] || missing+=("$arch/$f")
        done
    elif [ -f "$dir/vmlinuz" ]; then
        # Flat layout (legacy): assets/vmlinuz
        for f in vmlinuz initrd.img; do
            [ -f "$dir/$f" ] || missing+=("$f")
        done
        [ -f "$dir/rootfs.erofs" ] || missing+=("rootfs.erofs")
    else
        missing+=("vmlinuz (checked $dir/$arch/ and $dir/)")
    fi
    if [ ${#missing[@]} -gt 0 ]; then
        echo "Missing VM assets in $dir/: ${missing[*]}"
        echo "Building checked-in profile assets for $arch (requires docker)..."
        for profile in config/profiles/*/profile.toml; do
            just _build-assets "$(basename "$(dirname "$profile")")" "$arch"
        done
    fi

_pnpm-install:
    # CI=true suppresses pnpm's interactive "remove and reinstall
    # node_modules?" prompt, which hangs `just test` / `just smoke`
    # when the store layout drifts from the lockfile. Matches the
    # `CI=true pnpm install` already used in cross-compile and
    # test-install below.
    # Install every Node workspace used by local gates. CI has separate
    # jobs for docs/site/release-site, but `just test` and `just build-docs`
    # exercise those surfaces in this checkout too.
    for dir in frontend docs site release-site; do \
        (cd "$dir" && CI=true pnpm install --frozen-lockfile); \
    done

_release-site-pnpm-install:
    cd release-site && CI=true pnpm install --frozen-lockfile

_frontend: _pnpm-install
    bash scripts/check-web-surface.sh frontend-build

_compile: _frontend _clean-stale
    cargo build -p capsem

_sign-release: _compile
    #!/bin/bash
    set -euo pipefail
    if [[ "$(uname -s)" != "Darwin" ]]; then
        echo "  [omit] codesign (Linux -- not needed, using KVM)"
        exit 0
    fi
    if [[ ! -r "{{entitlements}}" ]]; then
        echo "ERROR: {{entitlements}} not found or not readable."
        echo "       This file should be checked into the repo. Try: git checkout {{entitlements}}"
        exit 1
    fi
    codesign --sign - --entitlements {{entitlements}} --force {{binary}}

_pack-initrd:
    #!/bin/bash
    set -euo pipefail
    ROOT="{{justfile_directory()}}"
    # Find initrd: per-arch layout first, then flat layout
    arch=$(uname -m | sed 's/aarch64/arm64/;s/arm64/arm64/')
    if [ -f "$ROOT/{{assets_dir}}/$arch/initrd.img" ]; then
        INITRD="$ROOT/{{assets_dir}}/$arch/initrd.img"
    elif [ -f "$ROOT/{{assets_dir}}/initrd.img" ]; then
        INITRD="$ROOT/{{assets_dir}}/initrd.img"
    else
        echo "ERROR: initrd.img not found. Run 'just doctor fix' first."
        exit 1
    fi
    # Cross-compile guest binaries only if missing or source changed
    RELEASE_DIR="$ROOT/target/linux-agent/$arch"
    NEED_BUILD=false
    for b in capsem-pty-agent capsem-net-proxy capsem-dns-proxy capsem-mcp-server capsem-sysutil capsem-bench-rs; do
        if [ ! -f "$RELEASE_DIR/$b" ]; then
            NEED_BUILD=true
            break
        fi
    done
    # Also rebuild if any guest binary source is newer than its staged binary.
    if [ "$NEED_BUILD" = "false" ] && [ -f "$RELEASE_DIR/capsem-pty-agent" ]; then
        NEWEST_SRC=$(find "$ROOT/crates/capsem-agent" "$ROOT/crates/capsem-proto" -name '*.rs' -newer "$RELEASE_DIR/capsem-pty-agent" 2>/dev/null | head -1)
        if [ -n "$NEWEST_SRC" ]; then
            NEED_BUILD=true
        fi
    fi
    if [ "$NEED_BUILD" = "false" ] && [ -f "$RELEASE_DIR/capsem-bench-rs" ]; then
        NEWEST_SRC=$(find "$ROOT/crates/capsem-bench" -name '*.rs' -newer "$RELEASE_DIR/capsem-bench-rs" 2>/dev/null | head -1)
        if [ -n "$NEWEST_SRC" ]; then
            NEED_BUILD=true
        fi
    fi
    if [ "$NEED_BUILD" = "true" ]; then
        echo "=== Cross-compile agent ==="
        uv run capsem-builder agent config/docker/image --arch "$arch"
        echo ""
    else
        echo "=== Agent binaries up to date, no cross-compile needed ==="
    fi
    # The builder applies 0o555 after a fresh cross-compile. Reassert the same
    # invariant below for cached staging directories too: a cached binary may
    # have been replaced or have its mode changed between builds.
    echo "=== Repack initrd ==="
    WORKDIR=$(mktemp -d)
    cd "$WORKDIR"
    gzip -dc "$INITRD" | cpio -id 2>/dev/null
    cp "$ROOT/guest/artifacts/capsem-init" init
    chmod 755 init
    # Verify binaries exist before repacking
    RELEASE_DIR="$ROOT/target/linux-agent/$arch"
    for b in capsem-pty-agent capsem-net-proxy capsem-dns-proxy capsem-mcp-server capsem-sysutil capsem-bench-rs; do
        if [ ! -f "$RELEASE_DIR/$b" ]; then
            echo "ERROR: $b missing from $RELEASE_DIR"
            exit 1
        fi
        chmod 555 "$RELEASE_DIR/$b"
        rm -f "$b"
        cp "$RELEASE_DIR/$b" .
        chmod 555 "$b"
    done
    rm -f capsem-doctor
    cp "$ROOT/guest/artifacts/capsem-doctor" capsem-doctor
    chmod 555 capsem-doctor
    rm -f capsem-bench
    cp "$ROOT/guest/artifacts/capsem-bench" capsem-bench
    chmod 555 capsem-bench
    rm -rf capsem_bench
    cp -r "$ROOT/guest/artifacts/capsem_bench" capsem_bench
    find capsem_bench -name '__pycache__' -exec rm -rf {} + 2>/dev/null || true
    rm -f snapshots
    cp "$ROOT/guest/artifacts/snapshots" snapshots
    chmod 555 snapshots
    rm -rf diagnostics
    cp -r "$ROOT/guest/artifacts/diagnostics" diagnostics
    # Atomic write: shell `> "$INITRD"` is truncate-write-in-place on the
    # inode. `create_hash_assets.py` (run below) gives the unhashed
    # `initrd.img` a hash-named hardlink (e.g. `initrd-<hex16>.img`) that
    # shares the same inode. An in-place rewrite mutates that hardlink's
    # content too, so any concurrent VM mid-`VmConfig::build` reading the
    # old hash-named path sees new bytes that don't match the embedded
    # hash. Symptom: `hash mismatch for ...img: expected X, got Y` -- a
    # stress run hitting this loses two cycles per `_pack-initrd` race.
    # Write to a sibling tmp + atomic rename keeps the old inode (and
    # the old hash-named hardlink) intact until `_cleanup_stale` below
    # explicitly unlinks it.
    TMP="${INITRD}.tmp.$$"
    find . | cpio -o -H newc 2>/dev/null | gzip > "$TMP"
    mv "$TMP" "$INITRD"
    rm -rf "$WORKDIR"
    cd "$ROOT"
    ASSETS="$ROOT/{{assets_dir}}"
    # Generate B3SUMS + manifest.json through the same admin rail used by
    # corp/release builds. The Python builder generator is an internal
    # implementation detail, never a public install/package path.
    VERSION=$(grep '^version' "$ROOT/Cargo.toml" | head -1 | sed 's/.*"\(.*\)"/\1/')
    cargo run -p capsem-admin -- manifest generate "$ASSETS" --version "$VERSION"
    # Create hash-named copies so dev layout matches installed layout.
    python3 "$ROOT/scripts/create_hash_assets.py" "$ASSETS"
    # Force cargo to re-run build.rs so it picks up new manifest hashes
    touch "$ROOT/crates/capsem-app/build.rs"
    echo "initrd repacked (with agent + net-proxy + mcp-server + sysutil + doctor)"

_materialize-config:
    #!/bin/bash
    set -euo pipefail
    ROOT="{{justfile_directory()}}"
    bash "$ROOT/scripts/materialize-config.sh"

# One bootable local runtime: verified assets, the initrd repacked around the
# current guest binaries, and a materialized profile catalog. `test` and
# `smoke` both need exactly this before they can run anything against a VM, so
# they name it once instead of repeating the sequence.
_prepared-runtime: _check-assets _pack-initrd _materialize-config
