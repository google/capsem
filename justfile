# Capsem Justfile
#
# Internal helpers:
#   _ensure-dev-ready checks for .dev-setup sentinel, runs doctor if missing (auto first-run)
#   _install-tools  auto-installs rust targets, components, cargo tools
#   _check-assets   verifies VM assets exist, builds checked-in profiles if not
#   _pack-initrd    cross-compiles guest binaries + repacks initrd
#   _sign           builds host binaries + codesigns (macOS only, required for VZ)
#   _ensure-service kills any running service, launches a fresh one, waits for socket
#
# User-facing recipe chains:
#   shell            -> _check-assets + _pack-initrd + _materialize-config + _ensure-service (daily dev entry point)
#   ui               -> _ensure-dev-ready + _pnpm-install + run-service (service + Tauri dev hot-reload)
#   run-service      -> _check-assets + _pack-initrd + _materialize-config + _ensure-service (start daemon, idempotent)
#   exec +CMD        -> run-service (one-shot command in a fresh temp VM)
#   build-assets     -> _install-tools + _clean-stale + inline doctor (kernel + rootfs via capsem-admin)
#   build-ui         -> _pnpm-install (pnpm build + cargo build -p capsem-app, in lockstep)
#   docs             -> _pnpm-install (build docs + marketing site release docs surfaces)
#   run-ui *ARGS     -> build-ui (launch ./target/debug/capsem-app)
#   test-frontend    -> frontend check + vitest + production build
#   smoke            -> _install-tools + _pnpm-install + _check-assets + _pack-initrd + _materialize-config + _ensure-service
#                       (audit, full doctor, injection, integration, parallel pytest groups)
#   test             -> _install-tools + _clean-stale + _pnpm-install + _generate-settings
#                       + _check-assets + _pack-initrd + _materialize-config (everything: audit, cov, cross-compile,
#                       all web surfaces, python, injection, integration, bench, test-install)
#   bench            -> _ensure-dev-ready + _check-assets + _pack-initrd + _materialize-config + _ensure-service
#   test-gateway     -> (no deps; unit + mock UDS tests)
#   test-gateway-e2e -> _check-assets + _pack-initrd + _materialize-config + _sign (real service + VMs)
#   test-install     -> _build-host (Docker e2e: build .deb, dpkg -i, pytest)
#   install          -> _pnpm-install + _stamp-version + _check-assets + _pack-initrd + _materialize-config
#                       (release build + frontend + Tauri bundle + .pkg/.deb installer)
#   prepare-release  -> test + _stamp-version (commits an untagged candidate)
#   qualify-release  -> remote canonical Linux gate for the exact candidate SHA
#   cut-release      -> verifies exact-SHA qualification, then creates local tag
#   release [tag]    -> (waits for CI on a pushed tag)
#
# First-time dev readiness:
#   just doctor       (shows what's missing; `just doctor fix` auto-installs)
#   just build-assets <profile-id> (builds profile-owned kernel + rootfs via capsem-admin -- needs docker via Colima on macOS)
#
# Daily dev:          just shell         (service daemon + temp VM + shell, ~10s)
#                     just ui            (service + Tauri GUI with hot-reload)
#                     just exec "<cmd>"  (one-shot command in a temp VM)
# Local install:      just install       (build .pkg/.deb + install it)
# Releases:           just prepare-release; push main; just qualify-release;
#                     just cut-release   (only then push the immutable tag)
# Dep maintenance:    just update-deps   (cargo update + pnpm update)
#                     just update-prices (refresh genai-prices.json)
#                     just update-fixture <src> (rebuild test.db fixture)
# Debugging:          just logs, just sandbox-logs <id>, just list-sessions,
#                     just inspect-session [id], just query-session "SQL"
# Disk cleanup:       just clean         (nuke target/ + frontend build, ~100 GB)
#                     just clean all     (clean + docker prune)

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
dev-tui *ARGS:
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
ui: _ensure-dev-ready _pnpm-install run-service
    #!/bin/bash
    set -euo pipefail
    source {{justfile_directory()}}/scripts/lib/exec_lock.sh
    acquire_exec_lock "$HOME/.capsem/run/execution.lock"
    CAPSEM_ASSETS_DIR={{assets_dir}} cargo tauri dev --config crates/capsem-app/tauri.conf.json

# Frontend-only dev server with mock data (no Tauri/VM needed)
dev-frontend: _pnpm-install _generate-settings
    cd frontend && pnpm run dev

# Build the Tauri desktop app (capsem-app) with a fresh frontend bundle.
# IMPORTANT: the Tauri binary embeds frontend/dist at cargo compile time via
# tauri::generate_context!(), so rebuilding only the frontend has no effect
# on the running binary. This recipe keeps the two in lockstep.
#   just build-ui          # debug binary at ./target/debug/capsem-app
#   just build-ui release  # release binary at ./target/release/capsem-app
build-ui profile="debug": _pnpm-install _generate-settings
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
test-frontend: _pnpm-install _generate-settings
    bash scripts/check-web-surface.sh frontend

# Build both public documentation surfaces used by the release gate.
docs: _pnpm-install
    bash scripts/check-web-surface.sh docs
    bash scripts/check-web-surface.sh site

# Run the Tauri desktop app after a clean frontend+binary rebuild.
# Pass extra args after `--`: `just run-ui -- --connect <vm-id>`.
run-ui *ARGS: build-ui
    #!/bin/bash
    set -euo pipefail
    pkill -f "target/(debug|release)/capsem-app" 2>/dev/null || true
    sleep 1
    ./target/debug/capsem-app {{ARGS}}

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
build-kernel arch profile="":
    #!/bin/bash
    set -euo pipefail
    PROFILE_ARG="{{profile}}"
    if [[ -z "$PROFILE_ARG" ]]; then
        echo "ERROR: profile id required. Use: just build-kernel {{arch}} <profile-id>"
        exit 2
    fi
    just _install-tools
    CAPSEM_SKIP_ASSET_CHECK=1 CAPSEM_SKIP_KVM_CHECK=1 just doctor
    cargo run -p capsem-admin -- image build \
        --profile "config/profiles/${PROFILE_ARG}/profile.toml" \
        --config-root config \
        --output "{{assets_dir}}" \
        --arch "{{arch}}" \
        --template kernel \
        --clean
    just _docker-gc

# Build rootfs only for one profile/arch (CI-facing primitive).
build-rootfs arch profile="":
    #!/bin/bash
    set -euo pipefail
    PROFILE_ARG="{{profile}}"
    if [[ -z "$PROFILE_ARG" ]]; then
        echo "ERROR: profile id required. Use: just build-rootfs {{arch}} <profile-id>"
        exit 2
    fi
    just _install-tools
    CAPSEM_SKIP_ASSET_CHECK=1 CAPSEM_SKIP_KVM_CHECK=1 just doctor
    cargo run -p capsem-admin -- image build \
        --profile "config/profiles/${PROFILE_ARG}/profile.toml" \
        --config-root config \
        --output "{{assets_dir}}" \
        --arch "{{arch}}" \
        --template rootfs \
        --clean
    just _docker-gc

# VM asset rebuild (kernel + rootfs). Profile is mandatory. Optional second arg
# restricts to one arch.
build-assets profile="" arch="":
    #!/bin/bash
    set -euo pipefail
    PROFILE_ARG="{{profile}}"
    ARCH_ARG="{{arch}}"
    if [[ -z "$PROFILE_ARG" ]]; then
        echo "ERROR: profile id required. Use: just build-assets <profile-id> [arm64|x86_64]"
        exit 2
    fi
    just _install-tools
    just _clean-stale
    CAPSEM_SKIP_ASSET_CHECK=1 CAPSEM_SKIP_KVM_CHECK=1 just doctor
    ARGS=(
        --profile "config/profiles/${PROFILE_ARG}/profile.toml"
        --config-root config
        --output "{{assets_dir}}"
        --clean
    )
    if [[ -n "$ARCH_ARG" ]]; then
        ARGS+=(--arch "$ARCH_ARG")
    fi
    cargo run -p capsem-admin -- image build "${ARGS[@]}"
    just _docker-gc

# Run vulnerability audits (cargo audit + npm bulk advisory API). Fast standalone gate.
# `just test` runs these too; this recipe is a quick pre-push check.
audit: _install-tools _pnpm-install
    #!/bin/bash
    set -euo pipefail
    echo "=== cargo audit ==="
    cargo audit
    echo ""
    echo "=== npm bulk audit ==="
    python3 scripts/audit-pnpm-bulk.py --project-dir frontend
    echo ""
    echo "Audits clean."

# Update all dependencies (Rust + npm) to latest compatible versions
update-deps: _pnpm-install
    #!/bin/bash
    set -euo pipefail
    echo "=== Cargo update ==="
    cargo update
    echo ""
    echo "=== Frontend update ==="
    cd frontend && pnpm update
    echo ""
    echo "Done. Run 'just smoke' to verify nothing broke."

# Run ALL tests: Rust + frontend + Python + injection + integration + bench + cross-compile + install e2e. No shortcuts.
#
# Runs against an isolated CAPSEM_HOME under target/test-home/ so the suite
# never kills or mutates the user's locally installed capsem. The flock is
# still honored for multi-agent coordination but now lives inside the test
# home, not the shared ~/.capsem/run.
# Show the latest preserved test-artifacts directory after a red `just test`.
# Lists files + sizes and prints the `cat` hint -- saves digging through
# `ls -lt test-artifacts/` after a failure.
test-artifacts:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ ! -d test-artifacts ]; then
        echo "No test-artifacts/ directory yet -- nothing has failed."
        exit 0
    fi
    LATEST=$(ls -1t test-artifacts/ 2>/dev/null | head -1 || true)
    if [ -z "$LATEST" ]; then
        echo "test-artifacts/ is empty."
        exit 0
    fi
    DIR="test-artifacts/$LATEST"
    echo "Latest preserved failure: $DIR"
    echo
    echo "Top-level layout:"
    find "$DIR" -maxdepth 3 -type f -exec stat -f '  %z %N' {} \; 2>/dev/null \
        || find "$DIR" -maxdepth 3 -type f -printf '  %s %P\n'
    echo
    echo "Hint:"
    echo "  cat $DIR/.../service.log | less"
    echo "  cat $DIR/.../sessions/<vm>/process.log | less"

_bootstrap:
    sh {{justfile_directory()}}/bootstrap.sh -y

test: _bootstrap _install-tools _clean-stale _pnpm-install _generate-settings _check-assets _pack-initrd _materialize-config
    #!/bin/bash
    set -euo pipefail
    export CAPSEM_HOME="{{justfile_directory()}}/target/test-home/.capsem"
    export CAPSEM_RUN_DIR="$CAPSEM_HOME/run"
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

    # ---- Stage 0: release harness bootstrap --------------------------------
    # Prove the clean Linux install container can resolve and launch its test
    # runner before spending ~2 hours on builds, VMs, and package assembly.
    # Stage 7 still runs the complete real install suite; this is only the
    # cheap fail-fast proof of the harness itself.
    echo "=== Install harness preflight (clean container) ==="
    just _test-install-harness-preflight

    # ---- Stage 1: fast-fail (audits + lint + frontend) ---------------------
    # Cheap, independent, most-common failure class. Clippy (not cargo check)
    # is the Rust lint gate per CLAUDE.md -- it's a strict superset of check
    # and covers --all-targets. Keep the production frontend build before
    # clippy: capsem-app's Tauri context embeds frontend/dist at compile time.
    # `set -e` does not trip on failed background jobs, so aggregate with
    # FAIL=1.
    echo "=== Audits + lint + web surfaces ==="
    cargo audit & PID_CARGO_AUDIT=$!
    python3 scripts/audit-pnpm-bulk.py --project-dir frontend & PID_PNPM_AUDIT=$!
    uv run ruff check . & PID_RUFF=$!
    uv run ty check src/capsem & PID_TY=$!
    uv run capsem-builder validate-skills skills & PID_SKILLS=$!
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
    wait $PID_CARGO_AUDIT || { echo "cargo audit failed"; FAIL=1; }
    wait $PID_PNPM_AUDIT  || { echo "npm bulk audit failed";  FAIL=1; }
    wait $PID_CLIPPY      || { echo "cargo clippy failed (warnings = error)"; FAIL=1; }
    wait $PID_RUFF        || { echo "ruff check failed"; FAIL=1; }
    wait $PID_TY          || { echo "ty check failed"; FAIL=1; }
    wait $PID_SKILLS      || { echo "skill validation failed"; FAIL=1; }
    [ $FAIL -eq 0 ] || exit 1

    # ---- Stage 2: cross-arch agent cross-compile ----------------------------
    # _pack-initrd already built the host arch; this validates the non-host
    # arch compiles cleanly against musl, so a cross-arch regression surfaces
    # before the Docker-based cross-compile at Stage 7.
    echo "=== Cross-compile agent (both arches) ==="
    uv run capsem-builder agent config/docker/image

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

    # ---- Stage 5: Python pytest ---------------------------------------------
    # Dogfooding canary: 4 concurrent VMs. --dist=loadfile keeps per-file
    # fixtures on the same worker. Any concurrency flake here is a Capsem-side
    # bug.
    #
    # Tests marked `serial` are benchmark/timing probes. They run after the
    # n=4 canary so their numbers measure Capsem, not another benchmark file
    # stealing the same Apple VZ launch budget.
    #
    # --ignore=tests/capsem-recipes -- recipe meta-tests invoke `cargo build
    #   --workspace` via subprocess, which atomically replaces the codesigned
    #   binaries concurrent VM tests need. All their assertions are already
    #   covered by Stage 1 clippy + Stage 3 llvm-cov + Stage 4 _build-host.
    #   Still runnable standalone via `uv run pytest -m recipe`.
    # --ignore=tests/capsem-install -- install-suite tests also spawn `cargo
    #   build -p capsem` from within pytest. This directory is owned by
    #   Stage 7's `just test-install`, which runs it inside Docker with
    #   CAPSEM_DEB_INSTALLED=1 (the live-system opt-in tests respect).
    echo "=== Python: non-serial tests (n=4 parallel) ==="
    # CAPSEM_REQUIRE_ARTIFACTS=1: fail the suite if any of assets/<arch>/
    # manifest.json, initrd.img, entitlements.plist, or target/linux-agent/
    # <arch>/ is missing. Stages 1-4 already produced them (this recipe
    # depends on _check-assets + _pack-initrd + _sign); if anything is
    # absent it means an earlier stage silently dropped its output, and
    # we want that to fail loudly here rather than manifest as a pile of
    # individually-omitted tests whose absence goes unnoticed.
    CAPSEM_REQUIRE_ARTIFACTS=1 uv run python -m pytest tests/ -v --tb=short -n 4 --dist=loadfile \
        -m "not serial" \
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
        -v --tb=short -m serial

    echo "=== Python: Build chain and release tests (serial) ==="
    CAPSEM_REQUIRE_ARTIFACTS=1 uv run python -m pytest tests/capsem-build-chain/ tests/capsem-release/ -v --tb=short

    # ---- Stage 6: legacy VM scripts + bench ---------------------------------
    echo "=== Injection test ==="
    python3 scripts/injection_test.py --binary {{binary}} --assets {{assets_dir}}

    echo "=== Integration test ==="
    python3 scripts/integration_test.py --binary {{binary}} --assets {{assets_dir}}

    echo "=== Benchmarks ==="
    # Records /tmp/capsem-benchmark.json to benchmarks/capsem-bench/data_<ver>_<arch>.json
    # on every run so we accumulate a baseline. No gate yet -- will grow
    # per-category tolerances once ~5-10 clean runs are on disk per arch.
    CAPSEM_ASSETS_DIR={{assets_dir}} uv run python -m pytest tests/capsem-serial/test_capsem_bench_baseline.py -v --tb=short

    # ---- Stage 7: Docker e2e ------------------------------------------------
    echo "=== Cross-compile Linux release (Docker) ==="
    just cross-compile

    echo "=== Install e2e tests (Docker + systemd) ==="
    just test-install

    # ---- Stage 8: cleanup ---------------------------------------------------
    echo "=== Pruning stale build artifacts ==="
    just _clean-stale

# Build the capsem-host-builder Docker image (cached, only rebuilds changed layers).
# See docker/Dockerfile.host-builder for contents.
build-host-image:
    #!/bin/bash
    set -euo pipefail
    echo "=== Building capsem-host-builder image ==="
    docker build \
        -t capsem-host-builder:latest \
        -f docker/Dockerfile.host-builder \
        docker/

# Remove cross-compilation image and cached volumes.
_clean-host-image:
    #!/bin/bash
    set -euo pipefail
    docker rmi capsem-host-builder:latest 2>/dev/null || true
    docker rmi capsem-install-test:latest 2>/dev/null || true
    for vol in capsem-cargo-registry capsem-cargo-git capsem-host-target-arm64 capsem-host-target-x86_64 capsem-rustup capsem-install-target capsem-install-cargo capsem-install-rustup; do
        docker volume rm "$vol" 2>/dev/null || true
    done
    echo "Cleaned host builder image and volumes."

# Build the full Linux release in a container (agent + deb).
# Uses the pre-built capsem-host-builder image (just build-host-image).
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
cross-compile arch="": _clean-stale _check-assets _generate-settings
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
    # Ensure build image exists
    if ! docker image inspect capsem-host-builder:latest &>/dev/null; then
        echo "=== Build image not found, building... ==="
        just build-host-image
    fi
    # Sync container VM clock on macOS (prevents apt "not valid yet" errors)
    if [[ "$(uname -s)" = "Darwin" ]]; then
        NOW=$(date -u +%Y-%m-%dT%H:%M:%SZ)
        docker run --rm --privileged alpine date -s "$NOW" 2>/dev/null || true
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
               swap-dev-libs \$DPKG_ARCH && \
               echo '--- Build agent binaries ---' && \
               cargo build --release --target \$RUST_TARGET -p capsem-agent && \
               mkdir -p /cargo-target/linux-agent/\$TARGET_ARCH && \
               cp /cargo-target/\$RUST_TARGET/release/capsem-pty-agent /cargo-target/\$RUST_TARGET/release/capsem-mcp-server /cargo-target/\$RUST_TARGET/release/capsem-net-proxy /cargo-target/\$RUST_TARGET/release/capsem-dns-proxy /cargo-target/\$RUST_TARGET/release/capsem-sysutil /cargo-target/linux-agent/\$TARGET_ARCH/ && \
               echo '--- Build companion host binaries ---' && \
               cargo build --release --target \$RUST_TARGET -p capsem -p capsem-service -p capsem-process -p capsem-tui -p capsem-mcp -p capsem-mcp-aggregator -p capsem-mcp-builtin -p capsem-gateway -p capsem-tray -p capsem-admin && \
               echo '--- Build frontend ---' && \
               cd frontend && CI=true pnpm install && pnpm build && cd .. && \
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

    HOST_ARCH=$(uname -m | sed 's/aarch64/arm64/;s/x86_64/x86_64/')
    if [ "$(uname -s)" = "Linux" ] \
        && [ "$TARGET_ARCH" = "$HOST_ARCH" ] \
        && [ -r /dev/kvm ] && [ -w /dev/kvm ] \
        && [ -r /dev/vhost-vsock ] && [ -w /dev/vhost-vsock ]; then
        echo "=== Proving exact Debian package in systemd + KVM ==="
        CAPSEM_PROOF_DEB="$DEB" \
        CAPSEM_PROOF_MANIFEST_URL="$MANIFEST_URL" \
        CAPSEM_PROOF_MANIFEST_CHANNEL="$MANIFEST_CHANNEL" \
            just _prove-linux-deb
    elif [ "${CAPSEM_REQUIRE_LINUX_DEB_PROOF:-0}" = "1" ]; then
        echo "ERROR: exact Debian package proof requires native Linux KVM and vhost-vsock" >&2
        echo "       host=$(uname -s)/$HOST_ARCH target=$TARGET_ARCH kvm=$(test -r /dev/kvm -a -w /dev/kvm && echo ready || echo unavailable) vhost-vsock=$(test -r /dev/vhost-vsock -a -w /dev/vhost-vsock && echo ready || echo unavailable)" >&2
        exit 1
    else
        echo "Skipping exact Debian package proof (requires native Linux KVM; release qualification makes this mandatory)."
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
    cargo audit & AUDIT_PID=$!
    python3 scripts/audit-pnpm-bulk.py --project-dir frontend & PNPM_AUDIT_PID=$!
    (cd frontend && pnpm run check) & FE_CHECK_PID=$!
    FAIL=0
    wait $CLIPPY_PID     || { echo "cargo clippy failed"; FAIL=1; }
    wait $RUFF_PID       || { echo "ruff check failed"; FAIL=1; }
    wait $TY_PID         || { echo "ty check failed"; FAIL=1; }
    wait $SKILLS_PID     || { echo "skill validation failed"; FAIL=1; }
    wait $AUDIT_PID      || { echo "cargo audit failed";  FAIL=1; }
    wait $PNPM_AUDIT_PID || { echo "npm bulk audit failed";   FAIL=1; }
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

# Gateway unit + integration tests (no VM needed)
test-gateway:
    #!/bin/bash
    set -euo pipefail
    echo "=== Gateway: Rust unit tests ==="
    cargo test -p capsem-gateway -- --nocapture
    echo ""
    echo "=== Gateway: build binary ==="
    cargo build -p capsem-gateway
    echo ""
    echo "=== Gateway: Python integration tests (mock UDS) ==="
    uv run python -m pytest tests/capsem-gateway/ -v --tb=short -m "gateway and not e2e"
    echo ""
    echo "Gateway tests passed"

# Gateway E2E tests (requires capsem-service + VM assets)
test-gateway-e2e: _check-assets _pack-initrd _materialize-config _sign
    #!/bin/bash
    set -euo pipefail
    source {{justfile_directory()}}/scripts/lib/exec_lock.sh
    acquire_exec_lock "$HOME/.capsem/run/execution.lock"
    cargo build -p capsem-gateway {{host_crates}}
    echo "=== Gateway: E2E tests (real service + VMs) ==="
    uv run python -m pytest tests/capsem-gateway/ -v --tb=short -m "gateway and e2e"

# Local HTML coverage report across all Rust crates
coverage:
    #!/bin/bash
    set -euo pipefail
    cargo llvm-cov --workspace --bins --lib --tests --no-cfg-coverage --html
    echo "Coverage report: target/llvm-cov/html/index.html"
    open target/llvm-cov/html/index.html 2>/dev/null || true

# Run in-VM benchmarks (disk I/O, rootfs read, CLI startup, HTTP latency)
bench: _ensure-dev-ready _check-assets _pack-initrd _materialize-config _ensure-service
    #!/bin/bash
    set -euo pipefail
    source {{justfile_directory()}}/scripts/lib/exec_lock.sh
    acquire_exec_lock "$HOME/.capsem/run/execution.lock"
    echo "=== In-VM benchmarks (disk, rootfs, CLI, HTTP, protocol, snapshots) ==="
    CAPSEM_ASSETS_DIR={{assets_dir}} uv run python -m pytest tests/capsem-serial/test_capsem_bench_baseline.py -v --tb=short
    echo ""
    echo "=== Host-side benchmarks (lifecycle, fork) ==="
    uv run python -m pytest \
        tests/capsem-serial/test_lifecycle_benchmark.py \
        tests/capsem-serial/test_route_latency_benchmark.py \
        -v --tb=short -m serial

# Build the platform package (.pkg on macOS, .deb on Linux) and install it.
# Builds release binaries, frontend, and Tauri app. Asks for sudo to install.
# The postinstall script handles codesign, PATH, service registration, and service readiness.
install: _pnpm-install _stamp-version _check-assets _pack-initrd _materialize-config
    #!/bin/bash
    set -euo pipefail
    # Strip test-isolation env vars so the installer never bakes a transient
    # target/test-home path into the LaunchAgent / systemd unit. If the user
    # was just running `just test` in this shell and exports lingered, the
    # install would permanently embed a path that gets wiped on the next
    # test run. `capsem install` also refuses these vars defensively.
    unset CAPSEM_HOME CAPSEM_RUN_DIR CAPSEM_ASSETS_DIR
    source {{justfile_directory()}}/scripts/lib/exec_lock.sh
    acquire_exec_lock "$HOME/.capsem/run/execution.lock"
    VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')
    MANIFEST_URL="${CAPSEM_INSTALL_MANIFEST_URL:-https://release.capsem.org/assets/stable/manifest.json}"
    MANIFEST_CHANNEL="${CAPSEM_INSTALL_CHANNEL:-stable}"
    export CAPSEM_BUILD_TS=$(date +%y%m%d%H%M)
    echo "=== Building release binaries (build=$CAPSEM_BUILD_TS) ==="
    cargo build --release {{host_crates}}
    echo "=== Building frontend ==="
    cd frontend
    pnpm build
    cd ..
    # Load Tauri signing key if available (needed for updater artifacts).
    # If absent, disable updater artifacts via config override.
    TAURI_FLAGS=""
    if [ -f "private/tauri/capsem.key" ]; then
        export TAURI_SIGNING_PRIVATE_KEY=$(cat private/tauri/capsem.key)
        export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=$(cat private/tauri/password.txt 2>/dev/null || echo "")
    else
        TAURI_FLAGS="--config '{\"bundle\":{\"createUpdaterArtifacts\":false}}'"
    fi
    # Unload LaunchAgent first so macOS doesn't respawn while we install
    PLIST="$HOME/Library/LaunchAgents/com.capsem.service.plist"
    if [ -f "$PLIST" ]; then
        launchctl bootout "gui/$(id -u)" "$PLIST" 2>/dev/null || \
            launchctl unload "$PLIST" 2>/dev/null || true
    fi
    pkill -9 -x capsem-service 2>/dev/null || true
    pkill -9 -x capsem-gateway 2>/dev/null || true
    pkill -9 -x capsem-tray 2>/dev/null || true
    pkill -9 -x capsem-process 2>/dev/null || true
    pkill -9 -x capsem-app 2>/dev/null || true
    sleep 0.5
    rm -f "$HOME/.capsem/run/service.sock"
    rm -f "$HOME/.capsem/run/gateway.token"
    rm -f "$HOME/.capsem/run/gateway.port"
    OS=$(uname -s)
    if [ "$OS" = "Darwin" ]; then
        echo "=== Building Capsem.app ==="
        eval cargo tauri build --bundles app $TAURI_FLAGS
        echo "=== Assembling .pkg (v$VERSION) ==="
        bash scripts/build-pkg.sh \
            --manifest "$MANIFEST_URL" \
            "target/release/bundle/macos/Capsem.app" \
            "target/release" \
            "{{assets_dir}}" \
            "target/config" \
            "$VERSION"
        PKG="packages/Capsem-$VERSION.pkg"
        echo "=== Installing package ==="
        if [ "$(id -u)" -eq 0 ]; then
            installer -pkg "$PKG" -target /
        else
            sudo installer -pkg "$PKG" -target /
        fi
    else
        echo "=== Building .deb ==="
        eval cargo tauri build --bundles deb $TAURI_FLAGS
        DEB=$(ls target/release/bundle/deb/*.deb)
        bash scripts/repack-deb.sh --manifest "$MANIFEST_URL" "$DEB" "target/release" "target/config" "{{assets_dir}}"
        echo "=== Installing .deb ==="
        sudo dpkg -i "$DEB" 2>&1 || sudo apt-get install -f -y
    fi
    # Post-install health check
    echo "=== Verifying service health ==="
    HEALTHY=false
    for i in $(seq 1 30); do
        if [ -S "$HOME/.capsem/run/service.sock" ] && \
           curl -s --unix-socket "$HOME/.capsem/run/service.sock" --max-time 2 http://localhost/list >/dev/null 2>&1; then
            echo "Service is responding."
            HEALTHY=true
            break
        fi
        sleep 0.5
    done
    if [ "$HEALTHY" != "true" ]; then
        echo "WARNING: Service not responding after 15s."
        if [ "$OS" = "Darwin" ]; then
            echo "Check: ~/Library/Logs/capsem/service.log"
        else
            echo "Check: journalctl --user -u capsem"
        fi
    fi
    "$HOME/.capsem/bin/capsem" status
    "$HOME/.capsem/bin/capsem" debug
    echo "=== Verifying installed release contract ==="
    python3 scripts/verify-installed-release.py \
        --capsem "$HOME/.capsem/bin/capsem" \
        --manifest-url "$MANIFEST_URL" \
        --channel "$MANIFEST_CHANNEL" \
        --package-version "$VERSION"
    echo "=== Proving installed guest shell ==="
    python3 scripts/prove-installed-shell.py \
        --capsem "$HOME/.capsem/bin/capsem" \
        --marker CAPSEM_LOCAL_NATIVE_INSTALL_SHELL_OK \
        --session-name local-native-install-shell \
        --timeout 300
    if [ "$OS" = "Darwin" ]; then
        echo "=== Opening Capsem.app ==="
        open /Applications/Capsem.app
    fi
    echo "=== Pruning stale build artifacts ==="
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
    IMAGE="capsem-install-test"
    # Always refresh the base from its checked-in Dockerfile. Docker keeps
    # unchanged layers cached; merely checking whether the tag exists lets a
    # stale local image hide new CI prerequisites.
    just build-host-image
    check_install_image() {
        docker run --rm \
            -u capsem \
            -e UV_PROJECT_ENVIRONMENT=/home/capsem/.venv-install-test \
            -v "$PWD":/src:ro \
            "$IMAGE" \
            bash -lc 'set -e; sudo -n true; cd /src; cdxgen --version; source /src/scripts/doctor-linux.sh; linux_musl_toolchain_available; uv run python -m pytest --version'
    }
    docker build -t "$IMAGE" -f docker/Dockerfile.install-test .
    if ! check_install_image; then
        echo "Install-test image smoke check failed; rebuilding without Docker cache..." >&2
        docker build --no-cache -t "$IMAGE" -f docker/Dockerfile.install-test .
        check_install_image
    fi

test-install:
    #!/bin/bash
    # No _build-host dep: the container does its own `cargo build` (line ~847)
    # against the GTK/glib -dev libs baked into Dockerfile.host-builder.
    # Pre-building on the CI runner duplicated work and broke on Linux
    # runners that lack libglib2.0-dev/libgtk-3-dev (the failure mode that
    # masked the asset-URL bug for v1.0.1777065213).
    set -euo pipefail
    just _test-install-harness-preflight
    IMAGE="capsem-install-test"
    # The derived install-test image is FROM capsem-host-builder. The cleanup
    # trap and low-disk recovery may remove both images, so the standalone gate
    # must restore the base image before attempting to rebuild the derived one.
    if ! docker image inspect capsem-host-builder:latest >/dev/null 2>&1; then
        echo "Building missing capsem-host-builder base image..."
        just build-host-image
    fi
    # Build the Docker image if needed
    if ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
        echo "Building $IMAGE Docker image..."
        docker build -t "$IMAGE" -f docker/Dockerfile.install-test .
    fi
    HOST_UID=$(id -u)
    HOST_GID=$(id -g)
    CONTAINER="capsem-install-test"
    # Detach the previous gate container before inspecting or resetting its
    # persistent target volume. Otherwise Docker refuses the volume removal;
    # ignoring that failure allowed the cache to fill the entire Docker disk.
    docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
    # Durable disk cushion. Both checks are no-ops in the common case
    # (plenty of Colima headroom, cache under 25 GB) so they don't thrash
    # the build cache every run -- they only fire when we're about to
    # fail anyway.
    # (a) If Colima has <10 GB free on /var/lib/docker, reclaim images +
    #     build cache aggressively (no until= filter). Linux hosts do not need this.
    if command -v colima >/dev/null 2>&1 && colima status >/dev/null 2>&1; then
        FREE_GB=$(colima ssh -- df -BG /var/lib/docker </dev/null 2>/dev/null | awk 'NR==2{gsub("G","",$4); print $4}')
        if [[ "${FREE_GB:-}" =~ ^[0-9]+$ ]] && [ "$FREE_GB" -lt 10 ]; then
            echo "Low Colima disk (${FREE_GB} GB free) -- pruning images + build cache..."
            docker image prune -af >/dev/null 2>&1 || true
            docker builder prune -af >/dev/null 2>&1 || true
        fi
    fi
    # (b) If the persistent cargo-target volume has grown past 25 GB,
    #     reset it. It caches debug artifacts across runs, but every
    #     crate version bump leaves dead code behind and the volume
    #     grows unbounded otherwise.
    VOLUME_LINE=$(docker system df -v 2>/dev/null | grep "^capsem-install-target " || true)
    if [ -n "$VOLUME_LINE" ]; then
        VOLUME_SIZE=$(echo "$VOLUME_LINE" | awk '{print $NF}')
        VOLUME_GB=$(echo "$VOLUME_SIZE" | grep -oE '^[0-9]+' | head -1)
        if [[ "${VOLUME_GB:-}" =~ ^[0-9]+$ ]] && echo "$VOLUME_SIZE" | grep -q "GB$" && [ "$VOLUME_GB" -gt 25 ]; then
            echo "capsem-install-target is ${VOLUME_SIZE} -- resetting (>25 GB threshold)..."
            if ! docker volume rm capsem-install-target >/dev/null; then
                echo "Error: Failed to reset oversized capsem-install-target volume." >&2
                echo "Containers still attached to the gate-owned volume:" >&2
                docker ps -a --filter volume=capsem-install-target >&2 || true
                exit 1
            fi
        fi
    fi
    # Stable container name + preemptive rm -f handles any container leaked
    # by a previous run that aborted before reaching cleanup (e.g. cargo
    # SIGTERM under Colima OOM). The EXIT trap below guarantees cleanup on
    # any exit path of *this* run so the leak can't start over.
    cleanup() {
        docker exec "$CONTAINER" bash -c "chown -R $HOST_UID:$HOST_GID /src 2>/dev/null || true" >/dev/null 2>&1 || true
        docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
        just _docker-gc >/dev/null 2>&1 || true
    }
    trap cleanup EXIT
    echo "Starting systemd container..."
    docker run -d --name "$CONTAINER" \
        --privileged --cgroupns=host \
        -v /sys/fs/cgroup:/sys/fs/cgroup:rw \
        --tmpfs /run --tmpfs /tmp \
        -v "$PWD":/src \
        -v capsem-install-target:/cargo-target \
        -v capsem-install-cargo:/usr/local/cargo/registry \
        -v capsem-install-rustup:/usr/local/rustup \
        "$IMAGE" /usr/lib/systemd/systemd
    # Wait for systemd to be ready
    for i in $(seq 1 30); do
        if docker exec "$CONTAINER" systemctl is-system-running --wait 2>/dev/null | grep -qE "running|degraded"; then
            break
        fi
        sleep 0.5
    done
    # Fix ownership for capsem user builds. /usr/local/rustup is included
    # because rustup self-updates (triggered by rust-toolchain.toml's
    # channel = "stable") try to write /usr/local/rustup/tmp/, which is
    # root-owned in the baked image -- without this chown, cargo build as
    # the capsem user dies with `Permission denied (os error 13)`.
    docker exec "$CONTAINER" bash -c "mkdir -p /cargo-target && chown -R capsem:capsem /cargo-target /usr/local/cargo /usr/local/rustup"
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
        "cd /src && bash scripts/prepare-install-test-assets.sh"
    echo "Materializing runtime config..."
    docker exec -u capsem "$CONTAINER" bash -c \
        "cd /src && bash scripts/materialize-config.sh"
    echo "Repacking .deb with companion binaries..."
    docker exec -u capsem "$CONTAINER" bash -c \
        'cd /src && DEB=$(ls -t /cargo-target/debug/bundle/deb/*.deb | head -1) && bash scripts/repack-deb.sh --manifest "file://$PWD/assets/manifest.json" "$DEB" /cargo-target/debug target/config assets'
    echo "Installing .deb via dpkg..."
    docker exec "$CONTAINER" bash -c \
        "dpkg -i /cargo-target/debug/bundle/deb/*.deb 2>&1 || apt-get install -f -y"
    echo "Running install e2e tests..."
    docker exec -u capsem -e XDG_RUNTIME_DIR=/run/user/1000 -e CAPSEM_DEB_INSTALLED=1 -e CAPSEM_BIN_SRC=/cargo-target/debug "$CONTAINER" bash -c \
        "mkdir -p /home/capsem/tmp && cd /src && UV_PROJECT_ENVIRONMENT=/home/capsem/.venv-install-test TMPDIR=/home/capsem/tmp uv run python -m pytest tests/capsem-install/ -v --tb=short"
    echo "Running local release glow-up (install, channel switch, upgrade)..."
    docker exec -u capsem -e XDG_RUNTIME_DIR=/run/user/1000 "$CONTAINER" bash -c \
        'cd /src && DEB=$(ls -t /cargo-target/debug/bundle/deb/*.deb | head -1) && python3 scripts/local-release-glowup.py --input-deb "$DEB" --bin-dir /cargo-target/debug --assets-dir assets --config-root target/config --work-dir target/local-release-glowup'

# Dispatch one serialized release workflow and wait for publication.
# Usage: just release                       (latest tag on HEAD, stable)
#        just release v0.9.13 stable        (explicit stable release)
#        just release v0.9.14-nightly nightly
release tag="" channel="stable":
    #!/usr/bin/env bash
    set -euo pipefail
    CHANNEL="{{channel}}"
    case "$CHANNEL" in
        stable|nightly) ;;
        *)
            echo "Error: channel must be stable or nightly (got: $CHANNEL)"
            exit 1
            ;;
    esac
    if [ -n "{{tag}}" ]; then
        TAG="{{tag}}"
    else
        TAG=$(git tag --points-at HEAD 'v*' | sort -V | tail -1)
        if [ -z "$TAG" ]; then
            echo "Error: HEAD has no v* tag. Pass one explicitly: just release v0.9.13"
            exit 1
        fi
    fi
    case "$TAG" in
        v*) ;;
        *)
            echo "Error: release tag must start with v (got: $TAG)"
            exit 1
            ;;
    esac
    if ! git ls-remote --exit-code --tags origin "refs/tags/$TAG" >/dev/null 2>&1; then
        echo "Error: tag $TAG is not published to origin"
        echo "Push it first: git push origin $TAG"
        exit 1
    fi
    LOCAL_TAG_SHA=$(git rev-parse "$TAG^{commit}")
    REMOTE_TAG_SHA=$(git ls-remote --tags origin "refs/tags/$TAG" "refs/tags/$TAG^{}" | \
        awk '$2 ~ /\^\{\}$/ { peeled=$1 } $2 !~ /\^\{\}$/ { direct=$1 } END { print peeled ? peeled : direct }')
    if ! test "$LOCAL_TAG_SHA" = "$REMOTE_TAG_SHA"; then
        echo "Error: local $TAG resolves to $LOCAL_TAG_SHA but origin resolves to $REMOTE_TAG_SHA" >&2
        echo "Never dispatch a release for a mismatched tag." >&2
        exit 1
    fi
    python3 scripts/check-release-qualification.py --sha "$LOCAL_TAG_SHA"

    RUN_TITLE="Release $CHANNEL $TAG"
    echo "=== $RUN_TITLE ==="
    RUN_ID=$(gh run list --workflow=release.yaml --event workflow_dispatch --limit 50 \
        --json databaseId,displayTitle,status,conclusion \
        --jq ".[] | select(.displayTitle==\"$RUN_TITLE\") | .databaseId" | head -1)
    if [ -n "$RUN_ID" ]; then
        STATUS=$(gh run view "$RUN_ID" --json status --jq .status)
        CONCLUSION=$(gh run view "$RUN_ID" --json conclusion --jq .conclusion)
        if [ "$STATUS" = "completed" ] && [ "$CONCLUSION" = "success" ]; then
            echo "=== $RUN_TITLE already published ==="
            echo "https://github.com/google/capsem/releases/tag/$TAG"
            exit 0
        fi
        if [ "$STATUS" = "completed" ]; then
            RUN_ID=""
        fi
    fi
    if [ -z "$RUN_ID" ]; then
        gh workflow run release.yaml --ref "$TAG" \
            -f "tag=$TAG" \
            -f "channel=$CHANNEL"
        for _ in $(seq 1 30); do
            RUN_ID=$(gh run list --workflow=release.yaml --event workflow_dispatch --limit 50 \
                --json databaseId,displayTitle \
                --jq ".[] | select(.displayTitle==\"$RUN_TITLE\") | .databaseId" | head -1)
            [ -n "$RUN_ID" ] && break
            sleep 2
        done
    fi
    if [ -z "$RUN_ID" ]; then
        echo "Error: dispatched workflow did not appear for $RUN_TITLE"
        exit 1
    fi
    echo "CI run: $RUN_ID"
    STATUS=$(gh run view "$RUN_ID" --json status --jq .status)
    if [ "$STATUS" != "completed" ]; then
        echo "Waiting for CI..."
        gh run watch "$RUN_ID"
    fi
    CONCLUSION=$(gh run view "$RUN_ID" --json conclusion --jq .conclusion)
    if [ "$CONCLUSION" != "success" ]; then
        echo "Error: CI run $RUN_ID failed ($CONCLUSION)"
        echo "Check: gh run view $RUN_ID --log-failed"
        exit 1
    fi
    echo "=== $RUN_TITLE published ==="
    echo "https://github.com/google/capsem/releases/tag/$TAG"

# Stamp the version and commit an untagged candidate. The exact commit must be
# pushed and remotely qualified before cut-release is allowed to mint a tag.
prepare-release: test _stamp-version
    #!/usr/bin/env bash
    set -euo pipefail
    NEW=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')
    TAG="v${NEW}"
    TODAY=$(date +%Y-%m-%d)
    echo "=== Preparing untagged release candidate $TAG ==="
    # Stamp changelog: [Unreleased] -> [NEW] - TODAY
    awk -v new="$NEW" -v today="$TODAY" '
        { print }
        $0 == "## [Unreleased]" {
            print ""
            print "## [" new "] - " today
        }
    ' CHANGELOG.md > CHANGELOG.md.tmp
    mv CHANGELOG.md.tmp CHANGELOG.md
    # Extract latest release notes for the frontend boot screen
    uv run python3 scripts/extract-release-notes.py
    # Commit only. A candidate is deliberately not a release tag.
    git add Cargo.toml crates/capsem-app/tauri.conf.json pyproject.toml uv.lock CHANGELOG.md LATEST_RELEASE.md
    git commit -m "release candidate: v${NEW}"
    echo "Prepared untagged candidate $(git rev-parse HEAD)."
    echo "Qualify it with:"
    echo "  git push origin HEAD:main"
    echo "  just qualify-release"
    echo "  just cut-release"

# Dispatch and wait for the canonical remote gate on the exact untagged HEAD.
# This recipe never creates a tag, GitHub release, or channel mutation.
qualify-release:
    #!/usr/bin/env bash
    set -euo pipefail
    SHA=$(git rev-parse HEAD)
    git fetch origin main
    if ! test "$(git rev-parse origin/main)" = "$SHA"; then
        echo "Error: exact candidate $SHA is not origin/main" >&2
        echo "Push the ordinary candidate commit before qualification." >&2
        exit 1
    fi
    if [ -n "$(git tag --points-at HEAD 'v*')" ]; then
        echo "Error: release qualification accepts only an untagged candidate" >&2
        exit 1
    fi
    if python3 scripts/check-release-qualification.py --sha "$SHA"; then
        exit 0
    fi
    RUN_TITLE="Qualify release $SHA"
    gh workflow run release-qualification.yaml --ref main -f "sha=$SHA"
    RUN_ID=""
    for _ in $(seq 1 30); do
        RUN_ID=$(gh run list --workflow=release-qualification.yaml --commit "$SHA" --event workflow_dispatch --limit 20 \
            --json databaseId,displayTitle,headSha \
            --jq ".[] | select(.displayTitle==\"$RUN_TITLE\" and .headSha==\"$SHA\") | .databaseId" | head -1)
        [ -n "$RUN_ID" ] && break
        sleep 2
    done
    if [ -z "$RUN_ID" ]; then
        echo "Error: exact-SHA qualification run did not appear" >&2
        exit 1
    fi
    echo "Qualification run: $RUN_ID"
    gh run watch "$RUN_ID" --exit-status
    python3 scripts/check-release-qualification.py --sha "$SHA"

# Mint the immutable local tag only after GitHub proves this exact published
# commit passed release qualification. No stamping or commit happens here.
cut-release:
    #!/usr/bin/env bash
    set -euo pipefail
    SHA=$(git rev-parse HEAD)
    git fetch origin main
    if ! test "$(git rev-parse origin/main)" = "$SHA"; then
        echo "Error: HEAD $SHA is not the exact published origin/main candidate" >&2
        exit 1
    fi
    NEW=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')
    TAG="v${NEW}"
    if git show-ref --verify --quiet "refs/tags/$TAG"; then
        echo "Error: local tag $TAG already exists; never reuse or move release tags" >&2
        exit 1
    fi
    if git ls-remote --exit-code --tags origin "refs/tags/$TAG" >/dev/null 2>&1; then
        echo "Error: remote tag $TAG already exists; never reuse or move release tags" >&2
        exit 1
    fi
    python3 scripts/check-release-qualification.py --sha "$SHA"
    git tag "$TAG"
    echo "Created qualified local tag $TAG at $SHA."
    echo "Publish with:"
    echo "  git push origin $TAG"
    echo "  just release $TAG stable"

# Check dev tools and dependencies. Pass "fix" to auto-fix.
doctor fix="": _pnpm-install
    #!/bin/bash
    if [ "{{fix}}" = "fix" ]; then
        scripts/doctor-common.sh --fix
    else
        scripts/doctor-common.sh
    fi

# Clean all build artifacts and report freed space
# Clean build artifacts. Pass "all" to also prune docker images/volumes.
clean all="":
    #!/bin/bash
    set -euo pipefail
    BEFORE=$(du -sk . 2>/dev/null | cut -f1)
    echo "=== Cleaning Capsem build artifacts ==="
    if [ -d target ]; then
        TARGET_SIZE=$(du -sh target 2>/dev/null | cut -f1)
        echo "  target/          ${TARGET_SIZE}"
        cargo clean
    fi
    for dir in frontend/dist frontend/node_modules tmp coverage; do
        if [ -d "$dir" ]; then
            DIR_SIZE=$(du -sh "$dir" 2>/dev/null | cut -f1)
            echo "  ${dir}/  ${DIR_SIZE}"
            rm -rf "$dir"
        fi
    done
    # Explicit removal of the isolated test home (also swept by `cargo clean`
    # when target/ is rebuilt, but listed here so it's visible in the log).
    if [ -d target/test-home ]; then
        echo "  target/test-home/   $(du -sh target/test-home 2>/dev/null | cut -f1)"
        rm -rf target/test-home
    fi
    # Leftover /tmp/capsem-test-* dirs from python helpers/service.py.
    if compgen -G "/tmp/capsem-test-*" >/dev/null; then
        TMP_COUNT=$(ls -d /tmp/capsem-test-* 2>/dev/null | wc -l | tr -d ' ')
        echo "  /tmp/capsem-test-*  ($TMP_COUNT entries)"
        rm -rf /tmp/capsem-test-*
    fi
    # Backup assets dir the dev _ensure-service created when it first found a
    # real ~/.capsem/assets/ (replaced with a symlink to the repo assets).
    if [ -d "$HOME/.capsem/assets.installed" ]; then
        echo "  ~/.capsem/assets.installed"
        rm -rf "$HOME/.capsem/assets.installed"
    fi
    if [[ "{{all}}" == "all" ]]; then
        just _clean-host-image
        if command -v docker &>/dev/null; then
            echo ""
            echo "=== Docker cleanup ==="
            docker system prune -af --volumes
        fi
    fi
    AFTER=$(du -sk . 2>/dev/null | cut -f1)
    FREED_KB=$((BEFORE - AFTER))
    if [ "$FREED_KB" -gt 1048576 ]; then
        echo ""
        echo "Freed $((FREED_KB / 1048576)) GB"
    elif [ "$FREED_KB" -gt 1024 ]; then
        echo ""
        echo "Freed $((FREED_KB / 1024)) MB"
    fi

# Inspect session DB integrity and event summary (latest by default)
inspect-session *args='':
    python3 scripts/check_session.py {{args}}

# View capsem-service logs
logs:
    tail -f ~/.capsem/run/service.log

# View logs for a specific sandbox (process + serial)
sandbox-logs id:
    {{cli_binary}} logs {{id}}

# TODO(forensics): replace last-logs with forensic log viewer from forensic sprint

# List recent sessions with event counts per table
list-sessions *args='':
    python3 scripts/list_sessions.py {{args}}

# Run a SQL query against a session DB (latest with a DB by default, or pass session ID)
query-session sql session_id='':
    #!/bin/bash
    set -euo pipefail
    SESSIONS_DIR="$HOME/.capsem/sessions"
    SID="{{session_id}}"
    if [ -z "$SID" ]; then
        # Find latest session that still has a session.db (ignore vacuumed)
        SID=$(sqlite3 "$SESSIONS_DIR/main.db" \
          "SELECT id FROM sessions WHERE status != 'vacuumed' ORDER BY created_at DESC LIMIT 1" \
          2>/dev/null || true)
        # Fallback: try any session with a DB on disk
        if [ -z "$SID" ] || [ ! -f "$SESSIONS_DIR/$SID/session.db" ]; then
            for d in $(ls -1t "$SESSIONS_DIR" 2>/dev/null); do
                [ -f "$SESSIONS_DIR/$d/session.db" ] && SID="$d" && break
            done
        fi
    fi
    if [ -z "$SID" ]; then
        echo "No sessions with a session.db found" >&2; exit 1
    fi
    DB="$SESSIONS_DIR/$SID/session.db"
    if [ ! -f "$DB" ]; then
        echo "No session.db at $DB (session may be vacuumed)" >&2; exit 1
    fi
    echo "Session: $SID"
    sqlite3 -header -column "$DB" "{{sql}}"

# Update test fixture DB from a real session (scrubs API keys)
update-fixture src:
    #!/usr/bin/env bash
    set -euo pipefail
    src="{{src}}"
    dst="data/fixtures/test.db"
    pub="frontend/public/fixtures/test.db"
    # Checkpoint WAL so we get a single clean file
    sqlite3 "$src" "PRAGMA wal_checkpoint(TRUNCATE);"
    cp "$src" "$dst"
    # Scrub any leaked API keys (belt-and-suspenders)
    sqlite3 "$dst" "
        UPDATE net_events SET request_headers  = REPLACE(request_headers,  'x-api-key', 'x-api-key-REDACTED') WHERE request_headers  LIKE '%sk-%';
        UPDATE net_events SET request_headers  = REPLACE(request_headers,  'authorization', 'authorization-REDACTED') WHERE request_headers  LIKE '%Bearer%';
        UPDATE net_events SET request_body_preview  = '' WHERE request_body_preview  LIKE '%sk-%' OR request_body_preview  LIKE '%AIza%';
        UPDATE net_events SET response_body_preview = '' WHERE response_body_preview LIKE '%sk-%' OR response_body_preview LIKE '%AIza%';
    "
    # Verify no keys leaked
    count=$(sqlite3 "$dst" "SELECT COUNT(*) FROM (
        SELECT 1 FROM net_events WHERE request_headers  LIKE '%sk-ant-%' OR request_headers  LIKE '%AIza%'
        UNION ALL
        SELECT 1 FROM net_events WHERE request_body_preview LIKE '%sk-ant-%' OR request_body_preview LIKE '%AIza%'
        UNION ALL
        SELECT 1 FROM net_events WHERE response_body_preview LIKE '%sk-ant-%' OR response_body_preview LIKE '%AIza%'
    );")
    if [ "$count" -ne 0 ]; then
        echo "ERROR: Found $count rows with potential API keys -- aborting"
        exit 1
    fi
    # Remove WAL/SHM leftovers
    rm -f "$dst-wal" "$dst-shm"
    # Copy to frontend public
    cp "$dst" "$pub"
    echo "Updated fixture: $(sqlite3 "$dst" 'SELECT COUNT(*) FROM net_events') net_events, $(sqlite3 "$dst" 'SELECT COUNT(*) FROM model_calls') model_calls"

# Update model pricing data from pydantic/genai-prices
update-prices:
    tmp="$(mktemp)"; \
    curl -fsSL https://raw.githubusercontent.com/pydantic/genai-prices/main/prices/data.json -o "$tmp"; \
    python3 -m json.tool "$tmp" >/dev/null; \
    python3 scripts/update_genai_prices.py "$tmp" config/data/genai-prices.json; \
    rm -f "$tmp"
    @echo "Updated compact config/data/genai-prices.json from pydantic/genai-prices prices/data.json"

# Remove stale rootfs copies, orphan UDS sockets, and trim bloated incremental caches.
# See scripts/clean_stale.py for implementation (tested: tests/capsem-cleanup-script/).
_clean-stale:
    @uv run python3 scripts/clean_stale.py

# Auto-prune Docker after builds: stopped containers, dangling images, build cache >7d.
# Keeps named volumes (cross-compile cargo caches) and recent build cache for fast rebuilds.
_docker-gc:
    #!/bin/bash
    if ! command -v docker &>/dev/null; then exit 0; fi
    # Remove stopped containers
    CONTAINERS=$(docker container ls -aq --filter status=exited 2>/dev/null)
    if [ -n "$CONTAINERS" ]; then
        docker container rm $CONTAINERS >/dev/null 2>&1 || true
    fi
    # Remove unused images older than 72h
    docker image prune -af --filter until=72h >/dev/null 2>&1 || true
    # Prune build cache older than 72h
    docker builder prune -f --filter until=72h >/dev/null 2>&1 || true
    # Reclaim sparse disk space from Colima VM (fstrim punches holes in the raw disk)
    if command -v colima &>/dev/null && colima status &>/dev/null; then
        colima ssh -- sudo fstrim /mnt/lima-colima >/dev/null 2>&1 || true
    fi

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
            just build-assets "$(basename "$(dirname "$profile")")" "$arch"
        done
    fi

_pnpm-install:
    # CI=true suppresses pnpm's interactive "remove and reinstall
    # node_modules?" prompt, which hangs `just test` / `just smoke`
    # when the store layout drifts from the lockfile. Matches the
    # `CI=true pnpm install` already used in cross-compile and
    # test-install below.
    # Install every Node workspace used by local gates. CI has separate
    # jobs for docs/site/release-site, but `just test` and `just docs`
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
        echo "ERROR: initrd.img not found. Run 'just build-assets' first."
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
