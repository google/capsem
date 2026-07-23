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
#   smoke                  focused local integration gate
#   test                   the only release-qualification gate
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
release_minor := "5"

# Stamp version as 1.{release_minor}.{unix_timestamp} in Cargo.toml,
# tauri.conf.json, pyproject.toml, and the frozen Python lockfile.
_stamp-version:
    #!/bin/bash
    set -euo pipefail
    CURRENT=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')
    RELEASE_MINOR="{{release_minor}}"
    if [[ ! "$RELEASE_MINOR" =~ ^[0-9]+$ ]]; then
        echo "Invalid release_minor: $RELEASE_MINOR" >&2
        exit 1
    fi
    NEW="1.${RELEASE_MINOR}.$(date +%s)"
    echo "Stamping version: ${CURRENT} -> ${NEW}"
    sed_in_place() {
        sed -i.bak "$1" "$2"
        rm -f "$2.bak"
    }
    sed_in_place "s/^version = \"${CURRENT}\"/version = \"${NEW}\"/" Cargo.toml
    sed_in_place "s/\"version\": \"${CURRENT}\"/\"version\": \"${NEW}\"/" crates/capsem-app/tauri.conf.json
    sed_in_place "s/^version = \"${CURRENT}\"/version = \"${NEW}\"/" pyproject.toml
    # Keep the editable project metadata in the frozen lockfile on the exact
    # release version before cut-release creates its commit and tag.
    uv lock --offline

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
# `just test` and `just smoke` can run against an isolated test home
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
shell: _check-assets _pack-initrd _materialize-config _ensure-service
    #!/bin/bash
    set -euo pipefail
    source {{justfile_directory()}}/scripts/lib/exec_lock.sh
    acquire_exec_lock "$HOME/.capsem/run/execution.lock"
    {{cli_binary}} shell

# Start capsem-service daemon (builds, signs, launches or reuses running instance)
run-service: _check-assets _pack-initrd _materialize-config _ensure-service

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
    #!/bin/bash
    set -euo pipefail
    ROOT="{{justfile_directory()}}"
    TEST_ROOT="$ROOT/target/ironbank-assets"
    HOST_ARCH=$(uname -m | sed 's/aarch64/arm64/;s/arm64/arm64/;s/amd64/x86_64/')
    case "$HOST_ARCH" in
        arm64|x86_64) ;;
        *)
            echo "ERROR: unsupported host architecture for asset boot proof: $HOST_ARCH" >&2
            exit 1
            ;;
    esac
    case "$HOST_ARCH" in
        arm64)
            CROSS_PLATFORM=linux/amd64
            CROSS_ARCH=x86_64
            ;;
        x86_64)
            CROSS_PLATFORM=linux/arm64
            CROSS_ARCH=arm64
            ;;
    esac
    echo "=== Ironbank $CROSS_ARCH container execution preflight ==="
    if ! docker run --rm --platform "$CROSS_PLATFORM" debian:bookworm-slim /bin/true; then
        echo "ERROR: Docker cannot execute $CROSS_PLATFORM containers." >&2
        if [[ "$(uname -s)" == "Darwin" ]]; then
            echo "Colima Rosetta may be configured but stale; run 'colima restart' and retry." >&2
        else
            echo "Install/register binfmt QEMU support and retry." >&2
        fi
        exit 1
    fi
    # Resolve the asset rail from the single checked-in storage policy. This
    # preserves the dual-architecture BuildKit cohort unless the daemon falls
    # below the declared capacity reserve.
    "$ROOT/scripts/ensure-docker-space.sh" assets
    rm -rf "$TEST_ROOT"
    mkdir -p "$TEST_ROOT"

    gate_pid_running() {
        local pid="$1"
        kill -0 "$pid" 2>/dev/null || return 1
        local state
        state=$(ps -o stat= -p "$pid" 2>/dev/null | awk '{print $1}')
        [[ -n "$state" && "$state" != Z* ]]
    }

    stop_gate_pidfile() {
        local pidfile="$1"
        local label
        label=$(basename "$pidfile")
        if [[ -f "$pidfile" ]]; then
            local pid
            pid=$(cat "$pidfile" 2>/dev/null || true)
            if [[ "$pid" =~ ^[0-9]+$ ]] && gate_pid_running "$pid"; then
                kill "$pid" 2>/dev/null || true
                for _ in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
                    gate_pid_running "$pid" || break
                    sleep 0.1
                done
                if gate_pid_running "$pid"; then
                    kill -9 "$pid" 2>/dev/null || true
                fi
                for _ in 1 2 3 4 5 6 7 8 9 10; do
                    gate_pid_running "$pid" || break
                    sleep 0.1
                done
                if gate_pid_running "$pid"; then
                    echo "ERROR: isolated asset gate $label process $pid did not exit" >&2
                    return 1
                fi
            fi
        fi
        rm -f "$pidfile"
    }

    stop_gate_service() {
        local run_dir="$1"
        # The gateway owns the fixed localhost port. Reap it explicitly before
        # stopping the service so the next profile cannot attach to an orphan
        # gateway whose UDS points at the previous profile's deleted run dir.
        stop_gate_pidfile "$run_dir/gateway.pid"
        stop_gate_pidfile "$run_dir/service.pid"
    }

    profile_paths=(config/profiles/*/profile.toml)

    # A hosted qualification runner has an observed hard lifetime below the
    # workflow's nominal timeout. Build two architecture lanes concurrently so
    # the complete four-cell profile/architecture matrix fits with enough time
    # left for pytest and installed-package proof. Each lane owns a distinct
    # Docker tag (capsem-*-<arch>) and isolated output root; this avoids both
    # tag collisions and concurrent manifest/current-symlink writes.
    build_arch_lane() {
        local arch="$1"
        for profile_path in "${profile_paths[@]}"; do
            local profile
            profile=$(basename "$(dirname "$profile_path")")
            local profile_root="$TEST_ROOT/$profile"
            local lane_assets="$profile_root/build-$arch"
            echo "=== Ironbank asset build lane: $profile ($arch) ==="
            just _build-image-template "$arch" "$profile" "$lane_assets" kernel
            just _build-image-template "$arch" "$profile" "$lane_assets" rootfs
            for logical_name in vmlinuz initrd.img rootfs.erofs obom.cdx.json software-inventory.json; do
                local artifact="$lane_assets/$arch/$logical_name"
                if [[ ! -s "$artifact" ]]; then
                    echo "ERROR: asset build did not produce non-empty $artifact" >&2
                    return 1
                fi
            done
        done
    }

    ARM64_BUILD_PID=""
    X86_64_BUILD_PID=""
    ARM64_BUILD_LOG="$TEST_ROOT/build-arm64.log"
    X86_64_BUILD_LOG="$TEST_ROOT/build-x86_64.log"
    cleanup_asset_builds() {
        local pid
        for pid in "$ARM64_BUILD_PID" "$X86_64_BUILD_PID"; do
            if [[ "$pid" =~ ^[0-9]+$ ]] && gate_pid_running "$pid"; then
                kill "$pid" 2>/dev/null || true
            fi
        done
        wait 2>/dev/null || true
        bash "$ROOT/scripts/cleanup-docker-containers-by-mount.sh" "$TEST_ROOT" || true
    }
    report_asset_lane_failure() {
        local arch="$1"
        local status="$2"
        local log="$3"
        echo "ERROR: Ironbank $arch asset-build lane failed (exit $status)" >&2
        if [[ -f "$log" ]]; then
            echo "--- tail of $log ---" >&2
            tail -n 200 "$log" >&2
            echo "--- complete log: $log ---" >&2
        else
            echo "ERROR: expected lane log is missing: $log" >&2
        fi
    }
    trap cleanup_asset_builds EXIT
    ( set -o pipefail; build_arch_lane arm64 2>&1 | tee "$ARM64_BUILD_LOG" ) &
    ARM64_BUILD_PID=$!
    ( set -o pipefail; build_arch_lane x86_64 2>&1 | tee "$X86_64_BUILD_LOG" ) &
    X86_64_BUILD_PID=$!
    ARM64_BUILD_STATUS=0
    X86_64_BUILD_STATUS=0
    wait "$ARM64_BUILD_PID" || ARM64_BUILD_STATUS=$?
    wait "$X86_64_BUILD_PID" || X86_64_BUILD_STATUS=$?
    ARM64_BUILD_PID=""
    X86_64_BUILD_PID=""
    trap - EXIT
    if [[ "$ARM64_BUILD_STATUS" -ne 0 ]]; then
        report_asset_lane_failure "arm64" "$ARM64_BUILD_STATUS" "$ARM64_BUILD_LOG"
    fi
    if [[ "$X86_64_BUILD_STATUS" -ne 0 ]]; then
        report_asset_lane_failure "x86_64" "$X86_64_BUILD_STATUS" "$X86_64_BUILD_LOG"
    fi
    if [[ "$ARM64_BUILD_STATUS" -ne 0 || "$X86_64_BUILD_STATUS" -ne 0 ]]; then
        exit 1
    fi

    for profile_path in "${profile_paths[@]}"; do
        profile=$(basename "$(dirname "$profile_path")")
        profile_root="$TEST_ROOT/$profile"
        profile_assets="$profile_root/assets"
        profile_config="$profile_root/config"
        profile_home="$profile_root/home/.capsem"
        mkdir -p "$profile_assets"
        for arch in arm64 x86_64; do
            lane_assets="$profile_root/build-$arch"
            cp -a "$lane_assets/$arch" "$profile_assets/$arch"
        done
        cargo run -p capsem-admin -- manifest generate "$profile_assets"
        # The builders update `current` to the arch they most recently built.
        # The merged manifest generator selects one build arch, so restore the
        # physical-host alias before the host-architecture VM proof.
        ln -sfn "$HOST_ARCH" "$profile_assets/current"
        test "$(readlink "$profile_assets/current")" = "$HOST_ARCH"

        # The installed runtime resolves content-addressed filenames. Build
        # output intentionally uses canonical logical names, so materialize
        # the same zero-copy hash aliases that package/dev preparation uses.
        # Without this, startup falls through to a remote fetch for the
        # local-only asset version and the gate is no longer hermetic.
        python3 scripts/create_hash_assets.py "$profile_assets"
        cargo run -p capsem-admin -- manifest check "$profile_assets/manifest.json"
        manifest_uri=$(python3 - "$profile_assets/manifest.json" <<'PY'
    from pathlib import Path
    import sys
    print(Path(sys.argv[1]).resolve().as_uri())
    PY
        )
        for runtime_profile in config/profiles/*/profile.toml; do
            cargo run -p capsem-admin -- profile materialize \
                --profile "$runtime_profile" \
                --config-root config \
                --manifest "$manifest_uri" \
                --assets-dir "$profile_assets" \
                --output-root "$profile_config" \
                --arch "$HOST_ARCH"
        done

        mkdir -p "$profile_home"
        # Keep AF_UNIX paths below macOS SUN_LEN. TEST_ROOT is intentionally
        # descriptive and too long once the gateway appends its session path.
        # Create runtime state only when the shell proof starts; durable
        # failure evidence is copied under target/ironbank-assets on exit.
        profile_run=$(mktemp -d /tmp/capsem-a.XXXXXX)
        marker="CAPSEM_ASSET_${profile//-/_}_${HOST_ARCH}_SHELL_OK"
        cleanup_asset_profile() {
            status=$?
            stop_gate_service "$profile_run"
            if [[ $status -ne 0 ]]; then
                rm -rf "$profile_root/run-failure"
                mkdir -p "$profile_root/run-failure"
                cp -R "$profile_run"/. "$profile_root/run-failure"/ 2>/dev/null || true
            fi
            rm -rf "$profile_run"
            return "$status"
        }
        trap cleanup_asset_profile EXIT
        CAPSEM_HOME="$profile_home" \
        CAPSEM_RUN_DIR="$profile_run" \
        CAPSEM_ASSETS_DIR="$profile_assets" \
        CAPSEM_PROFILES_DIR="$profile_config/profiles" \
            python3 scripts/prove-installed-shell.py \
                --capsem target/debug/capsem \
                --marker "$marker" \
                --session-name "asset-${profile}-${HOST_ARCH}" \
                --profile "$profile" \
                --timeout 300
        stop_gate_service "$profile_run"
        rm -rf "$profile_run"
        trap - EXIT
    done

    echo "Ironbank VM asset build and boot gate passed for every profile and architecture."

# Run ALL tests: Rust + frontend + Python + injection + integration + bench + cross-compile + install e2e. No shortcuts.
#
# Runs against an isolated CAPSEM_HOME under target/test-home/ so the suite
# never kills or mutates the user's locally installed capsem. The flock is
# still honored for multi-agent coordination but now lives inside the test
# home, not the shared ~/.capsem/run.
_bootstrap:
    sh {{justfile_directory()}}/bootstrap.sh -y

# Bind the complete gate to one clean committed tree. The candidate recipe may
# write ignored output under target/, but it must never change tracked or
# untracked source files or move to another commit while the gate is running.
test:
    #!/bin/bash
    set -euo pipefail
    if [ "$(uname -s)" = "Darwin" ] && [ -z "${CAPSEM_TEST_CAFFEINATED:-}" ]; then
        command -v caffeinate >/dev/null || {
            echo "macOS just test requires caffeinate to prevent an unattended release gate from sleeping" >&2
            exit 1
        }
        echo "=== Holding macOS awake for the complete candidate gate ==="
        exec caffeinate -dimsu env CAPSEM_TEST_CAFFEINATED=1 just test
    fi
    if [ -n "$(git status --porcelain --untracked-files=all)" ]; then
        echo "just test requires a clean working tree; commit the complete candidate first." >&2
        git status --short >&2
        exit 1
    fi
    TESTED_HEAD=$(git rev-parse HEAD)
    echo "=== Testing clean candidate $TESTED_HEAD ==="
    capture_candidate_failure() {
        status=$?
        if [ "$status" -ne 0 ]; then
            uv run python scripts/docker-storage-policy.py capture-failure \
                --rail default \
                --label "${TESTED_HEAD:0:12}" || true
        fi
        return "$status"
    }
    trap capture_candidate_failure EXIT
    scripts/with-gate-colima.sh just _test-candidate
    trap - EXIT
    test "$(git rev-parse HEAD)" = "$TESTED_HEAD" || {
        echo "candidate HEAD changed while just test was running" >&2
        exit 1
    }
    if [ -n "$(git status --porcelain --untracked-files=all)" ]; then
        echo "just test changed the candidate working tree" >&2
        git status --short >&2
        exit 1
    fi
    echo "=== Verified clean candidate $TESTED_HEAD ==="

# Require Docker headroom without discarding content-addressed compiler caches.
# Cargo validates cached artifacts against the exact candidate inputs; bounded
# reuse speeds forward candidates without weakening the clean-tree invariant.
_test-candidate: _bootstrap _bound-docker-test-storage _install-tools _clean-stale _pnpm-install _check-generated-settings _check-assets _pack-initrd _materialize-config
    #!/bin/bash
    set -euo pipefail
    export CAPSEM_HOME="{{justfile_directory()}}/target/test-home/.capsem"
    export CAPSEM_RUN_DIR="$CAPSEM_HOME/run"
    export CAPSEM_BENCHMARK_OUTPUT_ROOT="{{justfile_directory()}}/target/test-benchmarks"
    rm -rf "$CAPSEM_BENCHMARK_OUTPUT_ROOT"
    # Lockfile lives OUTSIDE $CAPSEM_HOME so it survives `rm -rf $CAPSEM_HOME`
    # below. Acquired BEFORE the wipe: if a second `just test` were to run
    # past this line, the first's fd would be pinned to an unlinked inode
    # and the second would flock a brand-new inode unchallenged.
    source {{justfile_directory()}}/scripts/lib/exec_lock.sh
    acquire_exec_lock "{{justfile_directory()}}/target/capsem-test-execution.lock"
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

    echo "=== Hardcoded profile/channel selection guard ==="
    bash scripts/check-hardcoded-release-selections.sh

    # ---- Stage 0: release harness bootstrap --------------------------------
    # Prove the clean Linux install container can resolve and launch its test
    # runner before spending ~2 hours on builds, VMs, and package assembly.
    # Stage 7 still runs the complete real install suite; this is only the
    # cheap fail-fast proof of the harness itself.
    echo "=== Install harness preflight (clean container) ==="
    just _test-install-harness-preflight
    # Neither final image is needed again until later rails: assets precede
    # Linux packaging, and the install tail is last. The preflight releases
    # the base; release the derived image here too. Bounded BuildKit keeps
    # reusable content-addressed layers without pinning ~12 GiB of images.
    uv run python scripts/docker-storage-policy.py release \
        --boundary after-install-preflight --rail install-preflight

    # ---- Stage 1: fast-fail (audits + lint + frontend) ---------------------
    # Cheap, independent, most-common failure class. Clippy (not cargo check)
    # is the Rust lint gate per CLAUDE.md -- it's a strict superset of check
    # and covers --all-targets. Keep the production frontend build before
    # clippy: capsem-app's Tauri context embeds frontend/dist at compile time.
    # `set -e` does not trip on failed background jobs, so aggregate with
    # FAIL=1.
    echo "=== Audits + lint + web surfaces ==="
    # Dependency advisories follow an external clock. The scheduled/manual
    # security-audit workflow owns the blocking signal; candidate qualification
    # reports a fresh result without letting an upstream database update make an
    # unrelated source commit red.
    (cargo audit || echo "::warning::cargo audit reported advisories; see the security-audit workflow") & PID_CARGO_AUDIT=$!
    (python3 scripts/audit-pnpm-bulk.py --project-dir frontend || echo "::warning::npm audit reported advisories; see the security-audit workflow") & PID_PNPM_AUDIT=$!
    uv run ruff check . & PID_RUFF=$!
    uv run ty check src/capsem & PID_TY=$!
    uv run capsem-builder validate-skills skills & PID_SKILLS=$!
    uv run python scripts/check_public_surface.py & PID_PUBLIC_SURFACE=$!
    FAIL=0
    if ! bash scripts/check-web-surface.sh frontend; then
        echo "frontend (check/test/build) failed"
        FAIL=1
    fi
    if ! bash scripts/check-web-surface.sh docs; then
        echo "docs build failed"
        FAIL=1
    fi
    if ! bash scripts/check-web-surface.sh site; then
        echo "marketing site build failed"
        FAIL=1
    fi
    if ! bash scripts/check-web-surface.sh release-site; then
        echo "release site (check/test/generated channel build) failed"
        FAIL=1
    fi
    cargo clippy --workspace --all-targets -- -D warnings & PID_CLIPPY=$!
    wait $PID_CARGO_AUDIT
    wait $PID_PNPM_AUDIT
    wait $PID_CLIPPY      || { echo "cargo clippy failed (warnings = error)"; FAIL=1; }
    wait $PID_RUFF        || { echo "ruff check failed"; FAIL=1; }
    wait $PID_TY          || { echo "ty check failed"; FAIL=1; }
    wait $PID_SKILLS      || { echo "skill validation failed"; FAIL=1; }
    wait $PID_PUBLIC_SURFACE || { echo "public surface approval failed"; FAIL=1; }
    [ $FAIL -eq 0 ] || exit 1

    # ---- Stage 2: cross-arch agent cross-compile ----------------------------
    # _pack-initrd already built the host arch; this validates the non-host
    # arch compiles cleanly against musl, so a cross-arch regression surfaces
    # before the Docker-based cross-compile at Stage 7.
    echo "=== Cross-compile agent (both arches) ==="
    uv run capsem-builder agent config/docker/image

    # ---- Stage 2b: Linux Rust platform parity ------------------------------
    # Native Linux runs exercise these cfg branches in Stage 3 below. A Mac
    # host must execute the same checked-in Linux runner inside the existing
    # host-builder image; otherwise Linux-only regressions can remain excluded
    # from the local canonical gate until exact-SHA CI.
    if [ "$(uname -s)" = "Darwin" ]; then
        echo "=== Linux Rust platform tests + coverage (Docker) ==="
        just _gate-linux-rust
        just _release-completed-linux-rust-target
        # The asset rail follows and needs the daemon reserve more than it
        # needs this disposable 6 GiB final image. Named Cargo/rustup volumes
        # remain hot; the completed BuildKit graph is bounded at the package
        # ownership boundary after the asset rail has consumed it.
        uv run python scripts/docker-storage-policy.py release \
            --boundary after-linux-rust-builder --rail assets
    fi

    # ---- Stage 3: Rust tests + coverage -------------------------------------
    # Threshold is 65, not 100. Some files (uninstall, completions) are intentionally
    # at 0% because they're thin shells over OS/CLI primitives. Some defensive paths
    # (capsem-process IPC handlers, run_shell exit cleanup) only exercise under live
    # VM traffic and are covered by integration tests under tests/, not unit tests.
    # The floor exists to catch a "we deleted half the test suite" regression, not to
    # gate every honest defensive-code addition.
    echo "=== Rust: test suite with coverage ==="
    cargo llvm-cov --workspace --bins --lib --tests --no-cfg-coverage --fail-under-lines 65

    # ---- Stage 4: sign host binaries for VM tests ---------------------------
    echo "=== Sign binaries for integration tests ==="
    just _sign

    # ---- Stage 4b: full VM asset build + boot parity -----------------------
    # This is the canonical owner of the portable work performed by the VM
    # asset publication workflow. It rebuilds every profile for both published
    # architectures in isolation, validates the manifests and release payload,
    # then boots each profile-owned host-arch result and proves a guest shell.
    echo "=== VM assets: all profiles, both arches, real guest shell ==="
    just _gate-assets

    # ---- Stage 5: Python pytest ---------------------------------------------
    # Dogfooding canary: 4 concurrent VMs. --dist=loadfile keeps per-file
    # fixtures on the same worker. Host snapshot/restore is the one exception:
    # each xdist worker owns a separate service, while the production host has
    # one service and one service-scoped save/restore lock. Mirror the smoke
    # gate by running those files only after the parallel workers have exited.
    #
    # Tests marked `serial` are benchmark/timing probes. They run after the
    # n=4 canary so their numbers measure Capsem, not another benchmark file
    # stealing the same Apple VZ launch budget.
    #
    # --ignore=tests/capsem-recipes -- recipe meta-tests invoke `cargo build
    #   --workspace` via subprocess, which atomically replaces the codesigned
    #   binaries concurrent VM tests need. They run explicitly after every VM
    #   and installed-package proof, when replacing those binaries is safe.
    # --ignore=tests/capsem-install -- install-suite tests also spawn `cargo
    #   build -p capsem` from within pytest. This directory is owned by
    #   Stage 7's private install gate, which runs it inside Docker with
    #   CAPSEM_DEB_INSTALLED=1 (the live-system opt-in tests respect).
    HOST_SNAPSHOT_SERIAL=(
        "tests/capsem-mcp/test_state_transitions.py"
        "tests/capsem-service/test_svc_resume_paths.py"
        "tests/capsem-service/test_svc_suspend_corruption.py"
        "tests/capsem-service/test_svc_loop_device_after_resume.py"
    )
    HOST_SNAPSHOT_IGNORE_ARGS=()
    for snapshot_file in "${HOST_SNAPSHOT_SERIAL[@]}"; do
        HOST_SNAPSHOT_IGNORE_ARGS+=("--ignore=$snapshot_file")
    done

    echo "=== Python: non-serial tests (n=4 parallel) ==="
    # CAPSEM_REQUIRE_ARTIFACTS=1: fail the suite if any of assets/<arch>/
    # manifest.json, initrd.img, entitlements.plist, or target/linux-agent/
    # <arch>/ is missing. Stages 1-4 already produced them (this recipe
    # depends on _check-assets + _pack-initrd + _sign); if anything is
    # absent it means an earlier stage silently dropped its output, and
    # we want that to fail loudly here rather than manifest as a pile of
    # individually-omitted tests whose absence goes unnoticed.
    CAPSEM_REQUIRE_ARTIFACTS=1 uv run python -m pytest tests/ -v --tb=short --maxfail=1 -n 4 --dist=loadfile \
        -m "not serial" \
        "${HOST_SNAPSHOT_IGNORE_ARGS[@]}" \
        --ignore=tests/capsem-recipes \
        --ignore=tests/capsem-install \
        --ignore=tests/capsem-build-chain \
        --ignore=tests/capsem-release \
        --ignore=tests/test_release_package_binary_contract.py \
        --ignore=tests/test_release_profile_architecture_contract.py \
        --ignore=tests/test_release_profile_contract.py \
        --ignore=tests/test_release_site_generated_from_json.py \
        --ignore=tests/test_release_site_html_contract.py \
        --ignore=tests/test_release_site_review_regressions.py \
        --cov=src/capsem --cov-report=xml:codecov-python.xml --cov-fail-under=90

    echo "=== Python: host snapshot tests (serial) ==="
    CAPSEM_REQUIRE_ARTIFACTS=1 uv run python -m pytest \
        "${HOST_SNAPSHOT_SERIAL[@]}" \
        -v --tb=short --maxfail=1 -m "not serial"

    echo "=== Python: release site shared-dist tests (serial) ==="
    CAPSEM_REQUIRE_ARTIFACTS=1 uv run python -m pytest \
        tests/test_release_package_binary_contract.py \
        tests/test_release_profile_architecture_contract.py \
        tests/test_release_profile_contract.py \
        tests/test_release_site_generated_from_json.py \
        tests/test_release_site_html_contract.py \
        tests/test_release_site_review_regressions.py \
        -v --tb=short

    echo "=== Python: serial timing and benchmark tests ==="
    CAPSEM_REQUIRE_ARTIFACTS=1 uv run python -m pytest \
        tests/capsem-serial/ \
        tests/ironbank/test_route_health.py \
        -v --tb=short -m serial -k 'not test_capsem_bench_baseline'

    echo "=== Python: Build chain and release tests (serial) ==="
    CAPSEM_REQUIRE_ARTIFACTS=1 uv run python -m pytest tests/capsem-build-chain/ tests/capsem-release/ -v --tb=short

    # ---- Stage 6: legacy VM scripts + bench ---------------------------------
    echo "=== Injection test ==="
    python3 scripts/injection_test.py --binary {{binary}} --assets {{assets_dir}}

    echo "=== Integration test ==="
    python3 scripts/integration_test.py --binary {{binary}} --assets {{assets_dir}}

    echo "=== Benchmarks ==="
    # Gate-owned recordings stay under target/test-benchmarks so the candidate
    # tree remains byte-for-byte identical to the tested commit.
    CAPSEM_ASSETS_DIR={{assets_dir}} uv run python -m pytest tests/capsem-serial/test_capsem_bench_baseline.py -v --tb=short

    # ---- Stage 7: Docker e2e ------------------------------------------------
    echo "=== Cross-compile Linux releases (Docker, both arches) ==="
    just _cross-compile arm64
    uv run python scripts/docker-storage-policy.py release \
        --boundary after-package-arm64 --rail package
    just _release-deferred-install-target
    just _cross-compile x86_64
    uv run python scripts/docker-storage-policy.py release \
        --boundary after-package-x86_64 --rail package
    # capsem-host-builder is a dependency of both package builds. Release its
    # final tag only after the second/last consumer, never between assets and
    # package assembly. The reusable 24 GiB BuildKit cohort remains warm.
    just _release-completed-buildkit-graph

    # ---- Stage 7b: publishable host packages + host SBOM -------------------
    # Linux release packages above are real release-mode .debs. On macOS,
    # build the real release-mode app and .pkg with the production assembler,
    # then install and exercise that exact package in a disposable Tart Mac.
    if [ "$(uname -s)" = "Darwin" ]; then
        echo "=== macOS clean-VM package install and glow-up ==="
        python3 scripts/macos_release_glowup.py
    fi
    echo "=== Host package SBOM artifact ==="
    just _gate-host-package-sbom

    echo "=== Install e2e tests (Docker + systemd) ==="
    just _gate-install

    echo "=== Just recipe tests (post-VM, serial) ==="
    uv run python -m pytest tests/capsem-recipes/ -v --tb=short -m recipe

    # ---- Stage 8: cleanup ---------------------------------------------------
    echo "=== Pruning stale build artifacts ==="
    just _clean-stale
    # Reassert the reserve after the expensive tail while retaining the hot
    # compiler/toolchain cache for the next deliberate candidate.
    just _bound-docker-test-storage

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
    @uv run python {{justfile_directory()}}/scripts/docker-storage-policy.py clean --scope all

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
_release-completed-linux-rust-target:
    @uv run python {{justfile_directory()}}/scripts/docker-storage-policy.py release \
        --boundary after-linux-rust --rail assets

_release-completed-docker-rails:
    @uv run python {{justfile_directory()}}/scripts/docker-storage-policy.py release \
        --boundary after-assets --rail package

_release-completed-buildkit-graph:
    @uv run python {{justfile_directory()}}/scripts/docker-storage-policy.py release \
        --boundary after-packages --rail package

_release-completed-package-rails:
    @uv run python {{justfile_directory()}}/scripts/docker-storage-policy.py release \
        --boundary after-package-arm64 --rail install
    @uv run python {{justfile_directory()}}/scripts/docker-storage-policy.py release \
        --boundary after-package-x86_64 --rail install

_release-deferred-install-target:
    @uv run python {{justfile_directory()}}/scripts/docker-storage-policy.py release \
        --boundary before-packages --rail package

_cross-compile arch="": _clean-stale _check-assets _generate-settings
    #!/bin/bash
    set -euo pipefail
    ROOT="{{justfile_directory()}}"
    MANIFEST_URL="${CAPSEM_INSTALL_MANIFEST_URL:-https://release.capsem.org/assets/stable/manifest.json}"
    MANIFEST_CHANNEL="${CAPSEM_INSTALL_CHANNEL:-stable}"
    case "$MANIFEST_CHANNEL" in
        stable|nightly|corp) ;;
        *)
            echo "ERROR: CAPSEM_INSTALL_CHANNEL must be stable, nightly, or corp (got: $MANIFEST_CHANNEL)" >&2
            exit 1
            ;;
    esac
    # Default to host architecture
    if [ -z "{{arch}}" ]; then
        TARGET_ARCH=$(uname -m | sed 's/aarch64/arm64/;s/x86_64/x86_64/')
    else
        TARGET_ARCH="{{arch}}"
    fi
    if [ "$TARGET_ARCH" != "arm64" ] && [ "$TARGET_ARCH" != "x86_64" ]; then
        echo "ERROR: unsupported arch '$TARGET_ARCH' (arm64 or x86_64)"
        exit 1
    fi
    just _release-completed-docker-rails
    # A completed install target retains only top-level runtime binaries and
    # the previous package after its post-install purge. It cannot accelerate
    # this package build (or the later clean install rebuild), so release it
    # before sacrificing the new host-builder or reusable registries.
    just _release-deferred-install-target
    # The package rail owns one named policy. Keep the Rust base image and
    # BuildKit cohort warm across candidates; capacity failure reports the
    # explicit disk recommendation instead of silently creating a cold build.
    "$ROOT/scripts/ensure-docker-space.sh" package
    # Always run the cached image build so changes to the Dockerfile or helper
    # scripts cannot be hidden behind a stale local image.
    just _build-host-image
    "$ROOT/scripts/ensure-docker-space.sh" package
    # Sync container VM clock on macOS (prevents apt "not valid yet" errors)
    if [[ "$(uname -s)" = "Darwin" ]]; then
        python3 scripts/sync-container-clock.py
    fi
    # Map target arch to Rust triple, dpkg arch, and pkg-config paths
    case "$TARGET_ARCH" in
        x86_64)
            RUST_TARGET="x86_64-unknown-linux-gnu"
            DPKG_ARCH="amd64"
            PKG_CONFIG_PATH_CROSS="/usr/lib/x86_64-linux-gnu/pkgconfig:/usr/share/pkgconfig"
            ;;
        arm64)
            RUST_TARGET="aarch64-unknown-linux-gnu"
            DPKG_ARCH="arm64"
            PKG_CONFIG_PATH_CROSS="/usr/lib/aarch64-linux-gnu/pkgconfig:/usr/share/pkgconfig"
            ;;
    esac
    # Sync assets layout for Tauri build
    rm -rf assets/current
    if [ -d "assets/$TARGET_ARCH" ]; then cp -r "assets/$TARGET_ARCH" assets/current; fi
    # If the host has the real release signing keys under private/tauri/,
    # inject them into the container. Otherwise the container generates a
    # throwaway dev-only key inline -- the authoritative release keys
    # live in GitHub Actions secrets (TAURI_SIGNING_PRIVATE_KEY +
    # TAURI_SIGNING_PRIVATE_KEY_PASSWORD in
    # .github/workflows/release.yaml) and are only applied on publish.
    # Dev builds just need SOME key so `cargo tauri build` can complete.
    SIGNING_ARGS=()
    if [ -f "$ROOT/private/tauri/capsem.key" ] && [ -f "$ROOT/private/tauri/password.txt" ]; then
        TAURI_KEY=$(cat "$ROOT/private/tauri/capsem.key")
        TAURI_PWD=$(cat "$ROOT/private/tauri/password.txt")
        SIGNING_ARGS=(
            -e "TAURI_SIGNING_PRIVATE_KEY=$TAURI_KEY"
            -e "TAURI_SIGNING_PRIVATE_KEY_PASSWORD=$TAURI_PWD"
        )
    fi
    echo "=== Building Linux deb ($TARGET_ARCH via docker, target=$RUST_TARGET) ==="
    mkdir -p "$ROOT/dist"
    HOST_UID=$(id -u)
    HOST_GID=$(id -g)
    # The builder is deliberately build-only: package postinstall requires a
    # real systemd user session and must never be exercised in this container.
    # Record the exact copied package so stale dist/ artifacts cannot be proved.
    DEB_RECORD="$ROOT/dist/.cross-compile-$TARGET_ARCH-deb"
    rm -f "$DEB_RECORD"
    docker run --rm \
        ${SIGNING_ARGS[@]+"${SIGNING_ARGS[@]}"} \
        -e "TARGET_ARCH=$TARGET_ARCH" \
        -e "RUST_TARGET=$RUST_TARGET" \
        -e "DPKG_ARCH=$DPKG_ARCH" \
        -e "PKG_CONFIG_PATH=$PKG_CONFIG_PATH_CROSS" \
        -e "CAPSEM_INSTALL_MANIFEST_URL=$MANIFEST_URL" \
        -e "HOST_UID=$HOST_UID" \
        -e "HOST_GID=$HOST_GID" \
        -v "$ROOT:/src" \
        -v "capsem-cargo-registry:/usr/local/cargo/registry" \
        -v "capsem-cargo-git:/usr/local/cargo/git" \
        -v "capsem-host-target-$TARGET_ARCH:/cargo-target" \
        -v "capsem-rustup:/usr/local/rustup" \
        -w /src \
        capsem-host-builder:latest \
        bash -c "trap 'chown -R \"\$HOST_UID:\$HOST_GID\" /src/dist /src/frontend/node_modules /src/frontend/dist 2>/dev/null || true' EXIT && \
               echo '--- Verify pinned Rust target ---' && \
               rustup toolchain install 1.97.1 --profile minimal && \
               rustup target add --toolchain 1.97.1 \$RUST_TARGET && \
               rustup target list --toolchain 1.97.1 --installed | grep -Fx \"\$RUST_TARGET\" >/dev/null || { echo \"ERROR: pinned Rust 1.97.1 target \$RUST_TARGET is unavailable in the capsem-rustup cache\" >&2; exit 1; } && \
               echo '--- Build frontend ---' && \
               cd frontend && CI=true pnpm install && pnpm build && cd .. && \
               swap-dev-libs \$DPKG_ARCH && \
               echo '--- Build agent binaries ---' && \
               cargo build --release --target \$RUST_TARGET -p capsem-agent && \
               mkdir -p /cargo-target/linux-agent/\$TARGET_ARCH && \
               cp /cargo-target/\$RUST_TARGET/release/capsem-pty-agent /cargo-target/\$RUST_TARGET/release/capsem-mcp-server /cargo-target/\$RUST_TARGET/release/capsem-net-proxy /cargo-target/\$RUST_TARGET/release/capsem-dns-proxy /cargo-target/\$RUST_TARGET/release/capsem-sysutil /cargo-target/linux-agent/\$TARGET_ARCH/ && \
               echo '--- Build companion host binaries ---' && \
               cargo build --release --target \$RUST_TARGET -p capsem -p capsem-service -p capsem-process -p capsem-tui -p capsem-mcp -p capsem-mcp-aggregator -p capsem-mcp-builtin -p capsem-gateway -p capsem-tray -p capsem-admin && \
               echo '--- Resolve Tauri signing key ---' && \
               DEV_KEY=/cargo-target/dev-tauri-private && \
               if [ -z \"\${TAURI_SIGNING_PRIVATE_KEY:-}\" ]; then \
                   if [ ! -f \"\$DEV_KEY\" ]; then \
                       echo '    no host signing key; generating dev-only key (not for release distribution)' && \
                       cargo tauri signer generate --ci --force -w \"\$DEV_KEY\" -p 'dev' >/dev/null; \
                   else \
                       echo \"    reusing dev key at \$DEV_KEY\"; \
                   fi && \
                   export TAURI_SIGNING_PRIVATE_KEY=\$(cat \"\$DEV_KEY\") && \
                   export TAURI_SIGNING_PRIVATE_KEY_PASSWORD='dev'; \
               else \
                   echo '    using host-injected signing key'; \
               fi && \
               echo '--- Build Tauri app ---' && \
               rm -rf /cargo-target/\$RUST_TARGET/release/bundle/deb && \
               cd crates/capsem-app && cargo tauri build --target \$RUST_TARGET --bundles deb && cd ../.. && \
               echo '--- Repack Debian package ---' && \
               DEB=\$(ls -t /cargo-target/\$RUST_TARGET/release/bundle/deb/*.deb | head -n1) && \
               bash scripts/repack-deb.sh --manifest \"\$CAPSEM_INSTALL_MANIFEST_URL\" \"\$DEB\" \"/cargo-target/\$RUST_TARGET/release\" \"target/config\" \"assets\" && \
               echo '--- Validate artifacts ---' && \
               dpkg-deb --info \"\$DEB\" && \
               dpkg-deb --contents \"\$DEB\" | grep -E 'usr/bin/(capsem|capsem-service|capsem-process|capsem-tui|capsem-mcp|capsem-mcp-aggregator|capsem-mcp-builtin|capsem-gateway|capsem-tray|capsem-admin)\$' && \
               cp \"\$DEB\" /src/dist/ && \
               basename \"\$DEB\" > \"/src/dist/.cross-compile-\$TARGET_ARCH-deb\" && \
               cp /cargo-target/linux-agent/\$TARGET_ARCH/* /src/dist/"
    if [ ! -s "$DEB_RECORD" ]; then
        echo "ERROR: builder did not record the exact Debian package" >&2
        exit 1
    fi
    DEB_BASENAME=$(tr -d '\r\n' < "$DEB_RECORD")
    rm -f "$DEB_RECORD"
    case "$DEB_BASENAME" in
        *.deb) ;;
        *)
            echo "ERROR: invalid Debian package record: $DEB_BASENAME" >&2
            exit 1
            ;;
    esac
    if [ "$DEB_BASENAME" != "$(basename "$DEB_BASENAME")" ]; then
        echo "ERROR: Debian package record escaped dist/: $DEB_BASENAME" >&2
        exit 1
    fi
    DEB="$ROOT/dist/$DEB_BASENAME"
    test -f "$DEB"

    HOST_OS=$(uname -s)
    HOST_ARCH=$(uname -m | sed 's/aarch64/arm64/;s/x86_64/x86_64/')
    KVM_READY=0
    if [ -r /dev/kvm ] && [ -w /dev/kvm ] \
        && [ -r /dev/vhost-vsock ] && [ -w /dev/vhost-vsock ]; then
        KVM_READY=1
    fi
    PROOF_DECISION=$(bash "$ROOT/scripts/select-linux-deb-proof.sh" \
        "$HOST_OS" \
        "$HOST_ARCH" \
        "$TARGET_ARCH" \
        "$KVM_READY" \
        "${CAPSEM_REQUIRE_LINUX_DEB_PROOF:-0}")
    if [ "$PROOF_DECISION" = "prove" ]; then
        echo "=== Proving exact Debian package in systemd + KVM ==="
        CAPSEM_PROOF_DEB="$DEB" \
        CAPSEM_PROOF_MANIFEST_URL="$MANIFEST_URL" \
        CAPSEM_PROOF_MANIFEST_CHANNEL="$MANIFEST_CHANNEL" \
            just _prove-linux-deb
    else
        echo "Skipping exact Debian package proof for non-host or optional target ($HOST_OS/$HOST_ARCH -> $TARGET_ARCH)."
    fi
    echo ""
    echo "=== Artifacts ==="
    ls -lh "$ROOT/dist/"
    just _docker-gc

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

# Fast path: audit, doctor, injection, integration tests (no Docker, no cross-compile)
smoke: _install-tools _pnpm-install _check-assets _pack-initrd _materialize-config
    #!/bin/bash
    set -euo pipefail
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
    step "Rust clippy + audits + frontend lint (parallel)"
    # Clippy (superset of cargo check) is the lint gate per CLAUDE.md.
    # Frontend `pnpm run check` runs here too so a broken Svelte/TS type
    # fails smoke in seconds instead of only surfacing under `just test`.
    # Background jobs don't trip `set -e`, so aggregate via FAIL=1.
    cargo clippy --workspace --all-targets -- -D warnings & CLIPPY_PID=$!
    uv run ruff check . & RUFF_PID=$!
    uv run ty check src/capsem & TY_PID=$!
    uv run capsem-builder validate-skills skills & SKILLS_PID=$!
    uv run python scripts/check_public_surface.py & PUBLIC_SURFACE_PID=$!
    (cargo audit || echo "::warning::cargo audit reported advisories; see the security-audit workflow") & AUDIT_PID=$!
    (python3 scripts/audit-pnpm-bulk.py --project-dir frontend || echo "::warning::npm audit reported advisories; see the security-audit workflow") & PNPM_AUDIT_PID=$!
    (cd frontend && pnpm run check) & FE_CHECK_PID=$!
    FAIL=0
    wait $CLIPPY_PID     || { echo "cargo clippy failed"; FAIL=1; }
    wait $RUFF_PID       || { echo "ruff check failed"; FAIL=1; }
    wait $TY_PID         || { echo "ty check failed"; FAIL=1; }
    wait $SKILLS_PID     || { echo "skill validation failed"; FAIL=1; }
    wait $PUBLIC_SURFACE_PID || { echo "public surface approval failed"; FAIL=1; }
    wait $AUDIT_PID
    wait $PNPM_AUDIT_PID
    wait $FE_CHECK_PID   || { echo "pnpm check failed";   FAIL=1; }
    [ $FAIL -eq 0 ] || exit 1
    step_done
    step "capsem-doctor (in-VM diagnostics)"
    {{cli_binary}} doctor
    step_done
    step "Injection test"
    python3 scripts/injection_test.py --binary {{binary}} --assets {{assets_dir}}
    step_done
    step "Integration test"
    python3 scripts/integration_test.py --binary {{binary}} --assets {{assets_dir}}
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
    #!/bin/bash
    set -euo pipefail
    ROOT="{{justfile_directory()}}"
    MANIFEST_URL="${CAPSEM_PROOF_MANIFEST_URL:?exact package proof requires CAPSEM_PROOF_MANIFEST_URL}"
    MANIFEST_CHANNEL="${CAPSEM_PROOF_MANIFEST_CHANNEL:?exact package proof requires CAPSEM_PROOF_MANIFEST_CHANNEL}"
    case "$MANIFEST_CHANNEL" in
        stable|nightly|corp) ;;
        *)
            echo "ERROR: unsupported exact package proof channel: $MANIFEST_CHANNEL" >&2
            exit 1
            ;;
    esac
    DEB_INPUT="${CAPSEM_PROOF_DEB:?exact package proof requires CAPSEM_PROOF_DEB}"
    DEB_DIR=$(cd "$(dirname "$DEB_INPUT")" && pwd -P)
    DEB="$DEB_DIR/$(basename "$DEB_INPUT")"
    case "$DEB" in
        "$ROOT"/dist/*.deb) ;;
        *)
            echo "ERROR: exact Debian package proof only accepts dist/*.deb (got: $DEB)" >&2
            exit 1
            ;;
    esac
    test -f "$DEB"
    test -r /dev/kvm -a -w /dev/kvm
    test -r /dev/vhost-vsock -a -w /dev/vhost-vsock

    IMAGE="capsem-install-test"
    CONTAINER="capsem-qualified-deb-proof"
    RELATIVE_DEB="${DEB#$ROOT/}"
    CONTAINER_DEB="/src/$RELATIVE_DEB"
    EXPECTED_VERSION=$(dpkg-deb -f "$DEB" Version)
    test -n "$EXPECTED_VERSION"

    DEVICE_ARGS=(
        --device /dev/kvm
        --device /dev/vhost-vsock
    )
    if [ -r /dev/vsock ] && [ -w /dev/vsock ]; then
        DEVICE_ARGS+=(--device /dev/vsock)
    fi
    cleanup() {
        docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
    }
    trap cleanup EXIT
    cleanup

    echo "Starting clean systemd container for exact package proof..."
    docker run -d --name "$CONTAINER" \
        --privileged --cgroupns=host \
        --security-opt seccomp=unconfined \
        "${DEVICE_ARGS[@]}" \
        -v /sys/fs/cgroup:/sys/fs/cgroup:rw \
        --tmpfs /run --tmpfs /tmp \
        -v "$ROOT:/src:ro" \
        "$IMAGE" /usr/lib/systemd/systemd

    SYSTEMD_READY=false
    for _ in $(seq 1 60); do
        if docker exec "$CONTAINER" systemctl is-system-running --wait 2>/dev/null \
            | grep -qE 'running|degraded'; then
            SYSTEMD_READY=true
            break
        fi
        sleep 0.5
    done
    if [ "$SYSTEMD_READY" != "true" ]; then
        echo "ERROR: systemd did not become ready for exact Debian package proof" >&2
        docker logs "$CONTAINER" >&2 || true
        exit 1
    fi
    docker exec "$CONTAINER" test -r /dev/kvm -a -w /dev/kvm
    docker exec "$CONTAINER" test -r /dev/vhost-vsock -a -w /dev/vhost-vsock

    echo "Installing exact package: $DEB"
    docker exec -e CONTAINER_DEB="$CONTAINER_DEB" "$CONTAINER" \
        bash -c 'dpkg -i "$CONTAINER_DEB" 2>&1 || apt-get install -f -y'
    INSTALLED_STATE=$(docker exec "$CONTAINER" dpkg-query -W -f='${Status}' capsem)
    INSTALLED_VERSION=$(docker exec "$CONTAINER" dpkg-query -W -f='${Version}' capsem)
    test "$INSTALLED_STATE" = "install ok installed"
    test "$INSTALLED_VERSION" = "$EXPECTED_VERSION"
    for bin in \
        capsem \
        capsem-admin \
        capsem-app \
        capsem-gateway \
        capsem-mcp \
        capsem-mcp-aggregator \
        capsem-mcp-builtin \
        capsem-process \
        capsem-service \
        capsem-tray \
        capsem-tui; do
        docker exec "$CONTAINER" test -x "/usr/bin/$bin"
        if [ "$bin" != "capsem-app" ]; then
            docker exec "$CONTAINER" "/usr/bin/$bin" --version | grep -F "$EXPECTED_VERSION"
        fi
    done

    STATUS_OUTPUT=$(docker exec \
        -u capsem \
        -e HOME=/home/capsem \
        -e XDG_RUNTIME_DIR=/run/user/1000 \
        "$CONTAINER" /usr/bin/capsem status)
    printf '%s\n' "$STATUS_OUTPUT"
    grep -F "Installed: true" <<<"$STATUS_OUTPUT"
    grep -F "Running:   true" <<<"$STATUS_OUTPUT"
    grep -F "Service:   ok" <<<"$STATUS_OUTPUT"
    grep -F "Gateway:   ok" <<<"$STATUS_OUTPUT"
    PROFILE_COUNTS=$(sed -n 's/^Profiles:[[:space:]]*\([0-9][0-9]*\)\/\([0-9][0-9]*\) ready.*/\1 \2/p' <<<"$STATUS_OUTPUT" | head -n 1)
    if [ -z "$PROFILE_COUNTS" ]; then
        echo "ERROR: exact package status has no Profiles: ready count" >&2
        exit 1
    fi
    read -r READY_PROFILES TOTAL_PROFILES <<<"$PROFILE_COUNTS"
    if [ "$TOTAL_PROFILES" -le 0 ] || [ "$READY_PROFILES" -ne "$TOTAL_PROFILES" ]; then
        echo "ERROR: exact package profiles are not all ready: $READY_PROFILES/$TOTAL_PROFILES" >&2
        exit 1
    fi

    docker exec \
        -u capsem \
        -e HOME=/home/capsem \
        -e XDG_RUNTIME_DIR=/run/user/1000 \
        "$CONTAINER" \
        python3 /src/scripts/verify-installed-release.py \
            --capsem /usr/bin/capsem \
            --capsem-home /home/capsem/.capsem \
            --manifest-url "$MANIFEST_URL" \
            --channel "$MANIFEST_CHANNEL" \
            --package-version "$EXPECTED_VERSION"

    docker exec \
        -u capsem \
        -e HOME=/home/capsem \
        -e XDG_RUNTIME_DIR=/run/user/1000 \
        "$CONTAINER" \
        python3 /src/scripts/prove-installed-shell.py \
            --capsem /usr/bin/capsem \
            --marker CAPSEM_QUALIFIED_DEB_SHELL_OK \
            --session-name qualification-exact-deb-shell \
            --timeout 300
    echo "Exact Debian package proof passed: version=$EXPECTED_VERSION profiles=$READY_PROFILES/$TOTAL_PROFILES"

_test-install-harness-preflight:
    #!/bin/bash
    set -euo pipefail
    ROOT="{{justfile_directory()}}"
    IMAGE="capsem-install-test"
    # Always refresh the base from its checked-in Dockerfile. Docker keeps
    # unchanged layers cached; merely checking whether the tag exists lets a
    # stale local image hide new CI prerequisites.
    just _build-host-image
    check_install_image() {
        docker run --rm \
            -u capsem \
            -e UV_PROJECT_ENVIRONMENT=/home/capsem/.venv-install-test \
            -v "$PWD":/src:ro \
            "$IMAGE" \
            bash -lc 'set -e; sudo -n true; cd /src; cdxgen --version; source /src/scripts/doctor-linux.sh; linux_musl_toolchain_available; uv run python -m pytest --version; uv run python -m pytest -p no:cacheprovider -q tests/test_materialize_config_http.py'
    }
    docker build -t "$IMAGE" -f docker/Dockerfile.install-test .
    if ! check_install_image; then
        echo "Install-test image smoke check failed; rebuilding without Docker cache..." >&2
        docker build --no-cache -t "$IMAGE" -f docker/Dockerfile.install-test .
        check_install_image
    fi
    # Stage 0 retains the base for the two immediately-following cross-compile
    # lanes. Other callers execute only the verified derived image, so release
    # the separate ~6 GiB base tag before their package/runtime work.
    if [ "${CAPSEM_KEEP_HOST_BUILDER:-0}" != "1" ]; then
        uv run python "$ROOT/scripts/docker-storage-policy.py" release \
            --boundary after-linux-rust-builder --rail install-preflight
    fi

_gate-install:
    #!/bin/bash
    # No _build-host dep: the container does its own `cargo build` (line ~847)
    # against the GTK/glib -dev libs baked into Dockerfile.host-builder.
    # Pre-building on the CI runner duplicated work and broke on Linux
    # runners that lack libglib2.0-dev/libgtk-3-dev (the failure mode that
    # masked the asset-URL bug for v1.0.1777065213).
    set -euo pipefail
    ROOT="{{justfile_directory()}}"
    # A completed install target retains only top-level runtime binaries and
    # the previous package after the post-install purge below; it no longer
    # contains the dependency graph needed for the next build. Release that
    # inactive lower-priority state before sacrificing reusable builder layers.
    just _release-deferred-install-target
    # The verified derived image pins roughly 6 GiB until the install proof
    # finishes. Reserve that image budget before materializing it, bounding
    # private BuildKit state only if completed install state was insufficient.
    "$ROOT/scripts/ensure-docker-space.sh" install-preflight
    just _test-install-harness-preflight
    IMAGE="capsem-install-test"
    # The preflight has already built and executed this exact derived image;
    # its separate base tag was released to keep the daemon below its cap.
    HOST_UID=$(id -u)
    HOST_GID=$(id -g)
    CONTAINER="capsem-install-test"
    INSTALL_ASSETS_DIR="target/install-test-assets"
    INSTALL_CONFIG_DIR="target/install-test-config"
    HOST_ROSETTA_REGISTRATION=not_applicable
    cleanup() {
        docker exec "$CONTAINER" bash -c "chown -R $HOST_UID:$HOST_GID /src 2>/dev/null || true" >/dev/null 2>&1 || true
        docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
        uv run python "$ROOT/scripts/docker-storage-policy.py" release \
            --boundary after-install --rail install || true
        uv run python "$ROOT/scripts/docker-storage-policy.py" gc \
            --rail install || true
    }
    # Install preflight has already created the disposable image. Own its
    # cleanup before any later capacity, platform, or container check can fail.
    trap cleanup EXIT
    if [ "$(uname -s)" = "Darwin" ] \
        && command -v colima >/dev/null 2>&1 \
        && colima status >/dev/null 2>&1; then
        if ! colima ssh -- test -f /proc/sys/fs/binfmt_misc/rosetta >/dev/null 2>&1; then
            echo "ERROR: Colima Rosetta binfmt registration is missing before test-install" >&2
            echo "Run the canonical bootstrap to repair it before testing packages." >&2
            exit 1
        fi
        HOST_ROSETTA_REGISTRATION=required
    fi
    # Detach the previous gate container before inspecting or resetting its
    # persistent target volume. Otherwise Docker refuses the volume removal;
    # ignoring that failure allowed the cache to fill the entire Docker disk.
    docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
    just _release-completed-package-rails
    # Durable disk cushion. The shared gate retains hot BuildKit and Cargo
    # cache while requiring enough daemon-local room for the package rail.
    "$ROOT/scripts/ensure-docker-space.sh" install
    # The one-shot install target was released declaratively before this rail.
    # Dependency downloads live in separate retained registry/rustup volumes.
    # Stable container name + preemptive rm -f handles any container leaked
    # by a previous run that aborted before reaching cleanup (e.g. cargo
    # SIGTERM under Colima OOM). The EXIT trap above guarantees cleanup on any
    # exit path of this run so the leak cannot start over.
    echo "Starting systemd container..."
    docker run -d --name "$CONTAINER" \
        --privileged --cgroupns=host \
        -v /sys/fs/cgroup:/sys/fs/cgroup:rw \
        --tmpfs /run --tmpfs /tmp \
        -v "$PWD":/src \
        -v capsem-install-target:/cargo-target \
        -v capsem-install-cargo:/usr/local/cargo/registry \
        -v capsem-install-rustup:/usr/local/rustup \
        -v capsem-install-frontend-node-modules:/src/frontend/node_modules \
        -v capsem-install-frontend-dist:/src/frontend/dist \
        "$IMAGE" /usr/lib/systemd/systemd
    # Wait for systemd to be ready
    for i in $(seq 1 30); do
        if docker exec "$CONTAINER" systemctl is-system-running --wait 2>/dev/null | grep -qE "running|degraded"; then
            break
        fi
        sleep 0.5
    done
    # Fix ownership for capsem user builds. /usr/local/rustup is included
    # because rustup may install pinned components/targets into its
    # root-owned state -- without this chown, cargo build as
    # the capsem user dies with `Permission denied (os error 13)`.
    # Frontend dependencies and output must remain container-owned. A macOS
    # bind mount preserves host ownership even after the best-effort /src
    # chown below; pnpm can then install successfully while Vite fails later
    # when it creates node_modules/.vite. Dedicated volumes make both writes
    # behave exactly like the non-root Linux CI filesystem.
    docker exec "$CONTAINER" bash -c "mkdir -p /cargo-target /src/frontend/node_modules /src/frontend/dist && chown -R capsem:capsem /cargo-target /usr/local/cargo /usr/local/rustup && chown -R capsem:capsem /src/frontend/node_modules /src/frontend/dist"
    # On GitHub runners the bind-mounted /src is owned by uid 1001
    # (runner), but the container builds as uid 1000 (capsem). Anything
    # that tries to write into /src (pnpm/vite temp files, Tauri build.rs
    # generating context into OUT_DIR but traversing /src, cargo's lock
    # checks, etc.) hits EACCES. Chown the whole tree once up front.
    docker exec "$CONTAINER" bash -c "chown -R capsem:capsem /src 2>/dev/null || true"
    echo "Building host binaries..."
    docker exec -u capsem "$CONTAINER" bash -c \
        "cd /src && cargo build {{host_crates}}"
    echo "Building frontend..."
    docker exec -u capsem -e CI=true "$CONTAINER" bash -c \
        "cd /src/frontend && pnpm install && pnpm build"
    echo "Building Tauri .deb..."
    # Clear stale bundles before the build: /cargo-target is a persistent
    # Docker volume, and any previous version's .deb lingers there. The
    # subsequent `ls *.deb` pickup would otherwise match both the stale
    # and current files -- `ls -t | head -1` below is belt-and-braces for
    # the same class of bug.
    docker exec -u capsem "$CONTAINER" bash -c \
        "rm -f /cargo-target/debug/bundle/deb/*.deb"
    docker exec -u capsem "$CONTAINER" bash -c \
        "cd /src && cargo tauri build --debug --bundles deb --config '{\"bundle\":{\"createUpdaterArtifacts\":false}}'"
    echo "Preparing install-test asset manifest..."
    docker exec -u capsem "$CONTAINER" bash -c \
        "cd /src && rm -rf \"$INSTALL_ASSETS_DIR\" \"$INSTALL_CONFIG_DIR\" && CAPSEM_ASSETS_DIR=\"$INSTALL_ASSETS_DIR\" bash scripts/prepare-install-test-assets.sh"
    echo "Materializing runtime config..."
    docker exec -u capsem "$CONTAINER" bash -c \
        "cd /src && CAPSEM_ASSETS_DIR=\"$INSTALL_ASSETS_DIR\" CAPSEM_CONFIG_OUTPUT_ROOT=\"/src/$INSTALL_CONFIG_DIR\" bash scripts/materialize-config.sh"
    echo "Repacking .deb with companion binaries..."
    docker exec -u capsem "$CONTAINER" bash -c \
        "cd /src && DEB=\$(ls -t /cargo-target/debug/bundle/deb/*.deb | head -1) && bash scripts/repack-deb.sh --manifest \"file://\$PWD/$INSTALL_ASSETS_DIR/manifest.json\" \"\$DEB\" /cargo-target/debug \"$INSTALL_CONFIG_DIR\" \"$INSTALL_ASSETS_DIR\""
    echo "Installing .deb via dpkg..."
    docker exec "$CONTAINER" bash -c \
        "dpkg -i /cargo-target/debug/bundle/deb/*.deb 2>&1 || apt-get install -f -y"
    # The package is linked and installed, so compiler object graphs are no
    # longer needed by pytest or the glow-up. They account for several GiB in
    # the active target volume and the global cleaner correctly refuses to
    # touch a mounted volume. Release them here while retaining the top-level
    # linked binaries and exact package bundle consumed by the runtime proofs.
    docker exec "$CONTAINER" bash -c \
        "rm -rf /cargo-target/debug/incremental /cargo-target/debug/deps /cargo-target/debug/build /cargo-target/debug/.fingerprint /cargo-target/debug/examples"
    # Package/image assembly can consume the reserve measured at recipe start.
    # The remaining runtime-only tail needs far less room than compilation, but
    # still keeps a 12 GiB cushion so ENOSPC fails here with diagnostics instead
    # of deep inside a fixture after hours of otherwise-green release work.
    "$ROOT/scripts/ensure-docker-space.sh" install
    echo "Running install e2e tests..."
    docker exec -u capsem -e XDG_RUNTIME_DIR=/run/user/1000 -e CAPSEM_DEB_INSTALLED=1 -e CAPSEM_BIN_SRC=/cargo-target/debug -e CAPSEM_TEST_ASSET_MANIFEST="/src/$INSTALL_ASSETS_DIR/manifest.json" "$CONTAINER" bash -c \
        "mkdir -p /home/capsem/tmp && cd /src && UV_PROJECT_ENVIRONMENT=/home/capsem/.venv-install-test TMPDIR=/home/capsem/tmp uv run python -m pytest tests/capsem-install/ -v --tb=short"
    echo "Running local release glow-up (install, channel switch, upgrade)..."
    docker exec -u capsem -e XDG_RUNTIME_DIR=/run/user/1000 "$CONTAINER" bash -c \
        "cd /src && DEB=\$(ls -t /cargo-target/debug/bundle/deb/*.deb | head -1) && python3 scripts/local-release-glowup.py --input-deb \"\$DEB\" --bin-dir /cargo-target/debug --assets-dir \"$INSTALL_ASSETS_DIR\" --config-root \"$INSTALL_CONFIG_DIR\" --work-dir target/local-release-glowup"
    if [ "$HOST_ROSETTA_REGISTRATION" = "required" ] \
        && ! colima ssh -- test -f /proc/sys/fs/binfmt_misc/rosetta >/dev/null 2>&1; then
        echo "ERROR: systemd install container removed Colima's Rosetta binfmt registration" >&2
        exit 1
    fi

# Check dev tools and dependencies. Pass "fix" to auto-fix.
doctor fix="": _pnpm-install
    #!/bin/bash
    if [ "{{fix}}" = "fix" ]; then
        scripts/doctor-common.sh --fix
    else
        scripts/doctor-common.sh
    fi

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
    @uv run python {{justfile_directory()}}/scripts/docker-storage-policy.py gc

# Enforce release-rail headroom while preserving content-addressed Cargo,
# registry, rustup, and recent BuildKit caches that make forward fixes fast.
_bound-docker-test-storage:
    @uv run python {{justfile_directory()}}/scripts/docker-storage-policy.py release \
        --boundary candidate-boundary --rail default
    @bash {{justfile_directory()}}/scripts/ensure-docker-space.sh default candidate-boundary

# Explicit deep cleanup for a human-requested cold rebuild. The canonical gate
# deliberately does not call this recipe.
_clean-docker-test-targets:
    @uv run python {{justfile_directory()}}/scripts/docker-storage-policy.py clean \
        --scope working --rail default

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
