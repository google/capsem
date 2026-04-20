# Capsem Justfile
#
# Internal helpers:
#   _ensure-setup   checks for .dev-setup sentinel, runs doctor if missing (auto first-run)
#   _install-tools  auto-installs rust targets, components, cargo tools
#   _check-assets   verifies VM assets exist, runs build-assets if not
#   _pack-initrd    cross-compiles guest binaries + repacks initrd
#   _sign           builds host binaries + codesigns (macOS only, required for VZ)
#   _ensure-service kills any running service, launches a fresh one, waits for socket
#
# User-facing recipe chains:
#   shell            -> _check-assets + _pack-initrd + _ensure-service (daily dev entry point)
#   ui               -> _ensure-setup + _pnpm-install + run-service (service + Tauri dev hot-reload)
#   run-service      -> _check-assets + _pack-initrd + _ensure-service (start daemon, idempotent)
#   exec +CMD        -> run-service (one-shot command in a fresh temp VM)
#   build-assets     -> _install-tools + _clean-stale + inline doctor (kernel + rootfs via capsem-builder)
#   build-ui         -> _pnpm-install (pnpm build + cargo build -p capsem-app, in lockstep)
#   run-ui *ARGS     -> build-ui (launch ./target/debug/capsem-app)
#   smoke            -> _install-tools + _pnpm-install + _check-assets + _pack-initrd + _ensure-service
#                       (audit, doctor --fast, injection, integration, parallel pytest groups)
#   test             -> _install-tools + _clean-stale + _pnpm-install + _generate-settings
#                       + _check-assets + _pack-initrd (everything: audit, cov, cross-compile,
#                       frontend, python, injection, integration, bench, test-install)
#   bench            -> _ensure-setup + _check-assets + _pack-initrd + _ensure-service
#   test-gateway     -> (no deps; unit + mock UDS tests)
#   test-gateway-e2e -> _check-assets + _pack-initrd + _sign (real service + VMs)
#   test-install     -> _build-host (Docker e2e: build .deb, dpkg -i, pytest)
#   install          -> _pnpm-install + _stamp-version + _check-assets + _pack-initrd
#                       (release build + frontend + Tauri bundle + .pkg/.deb installer)
#   cut-release      -> test + _stamp-version (commits changelog, tags, pushes, waits for CI)
#   release [tag]    -> (waits for CI on a pushed tag)
#
# First-time setup:
#   just doctor       (shows what's missing; `just doctor fix` auto-installs)
#   just build-assets (builds kernel + rootfs via capsem-builder -- needs docker via Colima on macOS)
#
# Daily dev:          just shell         (service daemon + temp VM + shell, ~10s)
#                     just ui            (service + Tauri GUI with hot-reload)
#                     just exec "<cmd>"  (one-shot command in a temp VM)
# Local install:      just install       (build .pkg/.deb + install it)
# Releases:           just cut-release   (test + bump, tag, push, CI)
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
host_binaries := "target/debug/capsem target/debug/capsem-service target/debug/capsem-process target/debug/capsem-mcp target/debug/capsem-mcp-aggregator target/debug/capsem-mcp-builtin target/debug/capsem-gateway target/debug/capsem-tray"
assets_dir := "assets"
entitlements := "entitlements.plist"
host_crates := "-p capsem-service -p capsem-process -p capsem -p capsem-mcp -p capsem-mcp-aggregator -p capsem-mcp-builtin -p capsem-gateway -p capsem-tray"

# Stamp version as 1.0.{unix_timestamp} in Cargo.toml, tauri.conf.json, and pyproject.toml.
_stamp-version:
    #!/bin/bash
    set -euo pipefail
    CURRENT=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')
    NEW="1.0.$(date +%s)"
    echo "Stamping version: ${CURRENT} -> ${NEW}"
    sed -i '' "s/^version = \"${CURRENT}\"/version = \"${NEW}\"/" Cargo.toml
    sed -i '' "s/\"version\": \"${CURRENT}\"/\"version\": \"${NEW}\"/" crates/capsem-app/tauri.conf.json
    sed -i '' "s/^version = \"${CURRENT}\"/version = \"${NEW}\"/" pyproject.toml

# Compile all host binaries
_build-host:
    cargo build {{host_crates}}

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
    arch=$(uname -m)
    [[ "$arch" == "arm64" ]] || arch="x86_64"
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
    # Symlink <capsem_home>/assets -> repo assets so installed tools (MCP, CLI)
    # see the same repacked initrd as the dev service.
    ASSETS_LINK="$CAPSEM_HOME_DIR/assets"
    DEV_ASSETS="$(cd "{{assets_dir}}" && pwd)"
    if [ -L "$ASSETS_LINK" ]; then
        CURRENT=$(readlink "$ASSETS_LINK")
        if [ "$CURRENT" != "$DEV_ASSETS" ]; then
            ln -sfn "$DEV_ASSETS" "$ASSETS_LINK"
            echo "Updated $ASSETS_LINK -> $DEV_ASSETS"
        fi
    elif [ -d "$ASSETS_LINK" ]; then
        # Real directory from install -- replace with symlink for dev.
        # Only happens on the default ~/.capsem layout; test homes start empty.
        rm -rf "$ASSETS_LINK.installed"
        mv "$ASSETS_LINK" "$ASSETS_LINK.installed"
        ln -sfn "$DEV_ASSETS" "$ASSETS_LINK"
        echo "Saved $ASSETS_LINK.installed, symlinked $ASSETS_LINK -> $DEV_ASSETS"
    else
        mkdir -p "$CAPSEM_HOME_DIR"
        ln -sfn "$DEV_ASSETS" "$ASSETS_LINK"
        echo "Symlinked $ASSETS_LINK -> $DEV_ASSETS"
    fi
    echo "Starting capsem-service (CAPSEM_HOME=$CAPSEM_HOME_DIR)..."
    RUST_LOG=capsem=debug {{service_binary}} \
        --assets-dir {{assets_dir}}/$arch \
        --process-binary {{process_binary}} \
        --foreground &
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
ui: _ensure-setup _pnpm-install run-service
    #!/bin/bash
    set -euo pipefail
    LOCK_FILE="$HOME/.capsem/run/execution.lock"
    mkdir -p "$(dirname "$LOCK_FILE")"
    exec 3>"$LOCK_FILE"
    flock -n 3 || { echo "another agent holds the capsem execution lock ($LOCK_FILE); try again later"; exit 1; }
    CAPSEM_ASSETS_DIR={{assets_dir}} cargo tauri dev --config crates/capsem-app/tauri.conf.json

# Frontend-only dev server with mock data (no Tauri/VM needed)
dev-frontend: _pnpm-install
    cd frontend && pnpm run dev

# Build the Tauri desktop app (capsem-app) with a fresh frontend bundle.
# IMPORTANT: the Tauri binary embeds frontend/dist at cargo compile time via
# tauri::generate_context!(), so rebuilding only the frontend has no effect
# on the running binary. This recipe keeps the two in lockstep.
#   just build-ui          # debug binary at ./target/debug/capsem-app
#   just build-ui release  # release binary at ./target/release/capsem-app
build-ui profile="debug": _pnpm-install
    #!/bin/bash
    set -euo pipefail
    echo "=== Frontend build ==="
    cd frontend
    pnpm run build
    cd ..
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

# Run the Tauri desktop app after a clean frontend+binary rebuild.
# Pass extra args after `--`: `just run-ui -- --connect <vm-id>`.
run-ui *ARGS: build-ui
    #!/bin/bash
    set -euo pipefail
    pkill -f "target/(debug|release)/capsem-app" 2>/dev/null || true
    sleep 1
    ./target/debug/capsem-app {{ARGS}}

# Start service daemon + boot temporary VM + shell (~10s after first build)
shell: _check-assets _pack-initrd _ensure-service
    #!/bin/bash
    set -euo pipefail
    LOCK_FILE="$HOME/.capsem/run/execution.lock"
    mkdir -p "$(dirname "$LOCK_FILE")"
    exec 3>"$LOCK_FILE"
    flock -n 3 || { echo "another agent holds the capsem execution lock ($LOCK_FILE); try again later"; exit 1; }
    {{cli_binary}} shell

# Start capsem-service daemon (builds, signs, launches or reuses running instance)
run-service: _check-assets _pack-initrd _ensure-service

# Execute a command in a fresh temporary VM (auto-provisioned and destroyed)
# Usage: just exec "echo hello"   or   just exec "ls -la"
exec +CMD: run-service
    #!/bin/bash
    set -euo pipefail
    LOCK_FILE="$HOME/.capsem/run/execution.lock"
    mkdir -p "$(dirname "$LOCK_FILE")"
    exec 3>"$LOCK_FILE"
    flock -n 3 || { echo "another agent holds the capsem execution lock ($LOCK_FILE); try again later"; exit 1; }
    {{cli_binary}} run "{{CMD}}"


# VM asset rebuild (kernel + rootfs). Default: both arches. Pass arch to build one.
build-assets arch="": _install-tools _clean-stale
    #!/bin/bash
    set -euo pipefail
    CAPSEM_SKIP_ASSET_CHECK=1 just doctor
    if [[ -n "{{arch}}" ]]; then
        arches=("{{arch}}")
        echo "=== Cleaning assets for {{arch}} ==="
        rm -rf "{{assets_dir}}/{{arch}}"
    else
        arches=(arm64 x86_64)
        echo "=== Cleaning all assets ==="
        rm -rf "{{assets_dir}}/arm64" "{{assets_dir}}/x86_64"
        rm -f "{{assets_dir}}/manifest.json" "{{assets_dir}}/B3SUMS"
    fi
    for a in "${arches[@]}"; do
        echo "=== Building kernel for $a ==="
        uv run capsem-builder build guest/ --arch "$a" --template kernel --output "{{assets_dir}}/"
        echo ""
        echo "=== Building rootfs for $a ==="
        uv run capsem-builder build guest/ --arch "$a" --template rootfs --output "{{assets_dir}}/"
        echo ""
    done
    echo "=== Generating checksums ==="
    uv run python3 -c 'from pathlib import Path; from capsem.builder.docker import generate_checksums, get_project_version; v = get_project_version(Path(".")); generate_checksums(Path("{{assets_dir}}"), v); print(f"manifest.json generated (v{v})")'
    just _docker-gc

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
test: _install-tools _clean-stale _pnpm-install _generate-settings _check-assets _pack-initrd
    #!/bin/bash
    set -euo pipefail
    export CAPSEM_HOME="{{justfile_directory()}}/target/test-home/.capsem"
    export CAPSEM_RUN_DIR="$CAPSEM_HOME/run"
    rm -rf "$CAPSEM_HOME"
    mkdir -p "$CAPSEM_RUN_DIR" "$CAPSEM_HOME/sessions" "$CAPSEM_HOME/logs"
    LOCK_FILE="$CAPSEM_RUN_DIR/execution.lock"
    exec 3>"$LOCK_FILE"
    flock -n 3 || { echo "another agent holds the test execution lock ($LOCK_FILE); try again later"; exit 1; }

    echo "=== Dependency audit ==="
    # Real vulnerabilities must fail the build. Upstream-only advisories
    # that we cannot remediate live in .cargo/audit.toml with justification
    # per ID -- not swallowed here.
    cargo audit
    (cd frontend && pnpm audit)

    echo "=== Rust: warnings-as-errors (all crates) ==="
    cargo check --workspace

    echo "=== Rust: test suite with coverage (compiles + runs all tests) ==="
    cargo llvm-cov --workspace --no-cfg-coverage --fail-under-lines 70

    echo "=== Cross-compile agent ==="
    uv run capsem-builder agent

    echo "=== Frontend ==="
    # Each step on its own line so `set -e` aborts on failure. Chaining with
    # `&&` suppresses errexit for non-final commands in bash, which silently
    # swallowed frontend test failures.
    cd frontend
    pnpm run check
    pnpm run test
    pnpm run build
    cd ..

    echo "=== Sign binaries for integration tests ==="
    just _sign

    echo "=== Python: ALL tests (no marker exclusions, n=4 parallel) ==="
    # n=4 parallel: this is the dogfooding canary. We ship Capsem as a
    # multi-VM sandbox for AI agents -- if our own tests can't safely
    # boot 4 VMs concurrently, real users will hit the same bug. Any
    # concurrency flake here is a Capsem-side bug, not a test-tuning
    # problem. loadfile keeps tests in the same module on the same
    # worker so per-file fixtures are not re-init'd 4x.
    uv run python -m pytest tests/ -v --tb=short -n 4 --dist=loadfile \
        --cov=src/capsem --cov-report=xml:codecov-python.xml --cov-fail-under=90

    echo "=== Injection test ==="
    python3 scripts/injection_test.py --binary {{binary}} --assets {{assets_dir}}

    echo "=== Integration test ==="
    python3 scripts/integration_test.py --binary {{binary}} --assets {{assets_dir}}

    echo "=== Benchmarks ==="
    CAPSEM_ASSETS_DIR={{assets_dir}} {{binary}} "capsem-bench"

    echo "=== Cross-compile Linux release (Docker) ==="
    just cross-compile

    echo "=== Install e2e tests (Docker + systemd) ==="
    just test-install

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
    for vol in capsem-cargo-registry capsem-cargo-git capsem-host-target-arm64 capsem-host-target-x86_64 capsem-rustup-arm64 capsem-rustup-x86_64 capsem-install-target capsem-install-cargo; do
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
    # Load Tauri signing key if available
    TAURI_KEY=""
    TAURI_PWD=""
    if [ -f "$ROOT/private/tauri/capsem.key" ]; then
        TAURI_KEY=$(cat "$ROOT/private/tauri/capsem.key")
        TAURI_PWD=$(cat "$ROOT/private/tauri/password.txt")
    fi
    echo "=== Building Linux deb ($TARGET_ARCH via docker, target=$RUST_TARGET) ==="
    mkdir -p "$ROOT/dist"
    # KVM boot test: pass /dev/kvm if available (Linux host) or skip (macOS)
    KVM_FLAG=""
    if [ -e /dev/kvm ]; then
        KVM_FLAG="--device /dev/kvm"
    fi
    docker run --rm \
        $KVM_FLAG \
        -e "TAURI_SIGNING_PRIVATE_KEY=$TAURI_KEY" \
        -e "TAURI_SIGNING_PRIVATE_KEY_PASSWORD=$TAURI_PWD" \
        -e "TARGET_ARCH=$TARGET_ARCH" \
        -e "RUST_TARGET=$RUST_TARGET" \
        -e "DPKG_ARCH=$DPKG_ARCH" \
        -e "PKG_CONFIG_PATH=$PKG_CONFIG_PATH_CROSS" \
        -v "$ROOT:/src" \
        -v "capsem-cargo-registry:/usr/local/cargo/registry" \
        -v "capsem-cargo-git:/usr/local/cargo/git" \
        -v "capsem-host-target-$TARGET_ARCH:/cargo-target" \
        -w /src \
        capsem-host-builder:latest \
        bash -c "swap-dev-libs \$DPKG_ARCH && \
               echo '--- Build agent binaries ---' && \
               cargo build --release --target \$RUST_TARGET -p capsem-agent && \
               mkdir -p /cargo-target/linux-agent/\$TARGET_ARCH && \
               cp /cargo-target/\$RUST_TARGET/release/capsem-pty-agent /cargo-target/\$RUST_TARGET/release/capsem-mcp-server /cargo-target/\$RUST_TARGET/release/capsem-net-proxy /cargo-target/\$RUST_TARGET/release/capsem-sysutil /cargo-target/linux-agent/\$TARGET_ARCH/ && \
               echo '--- Build frontend ---' && \
               cd frontend && CI=true pnpm install && pnpm build && cd .. && \
               echo '--- Build Tauri app ---' && \
               cd crates/capsem-app && cargo tauri build --target \$RUST_TARGET --bundles deb && cd ../.. && \
               echo '--- Validate artifacts ---' && \
               dpkg-deb --info /cargo-target/\$RUST_TARGET/release/bundle/deb/*.deb && \
               cp /cargo-target/\$RUST_TARGET/release/bundle/deb/*.deb /src/dist/ && \
               cp /cargo-target/linux-agent/\$TARGET_ARCH/* /src/dist/ && \
               echo '--- Boot test ---' && \
               if [ -e /dev/kvm ] && [ \"\$TARGET_ARCH\" = \"\$(uname -m | sed 's/aarch64/arm64/')\" ]; then \
                   echo 'KVM available + native arch: running boot test' && \
                   dpkg -i /cargo-target/\$RUST_TARGET/release/bundle/deb/*.deb 2>/dev/null || apt-get install -f -y && \
                   timeout 120 python3 scripts/doctor_session_test.py --binary capsem --assets assets; \
               else \
                   echo 'Skipping boot test (no KVM or cross-arch -- CI will test)'; \
               fi"
    echo ""
    echo "=== Artifacts ==="
    ls -lh "$ROOT/dist/"
    just _docker-gc

# Generate settings-schema.json, defaults.json, mcp-tools.json, and mock-data.generated.ts
_generate-settings:
    #!/bin/bash
    set -euo pipefail
    LOG="target/build.log"
    mkdir -p target
    echo "[generate] $(date +%H:%M:%S) exporting MCP tool defs" >> "$LOG"
    cargo run --bin mcp_export 2>>"$LOG" > config/mcp-tools.json
    echo "[generate] $(date +%H:%M:%S) generating schema + defaults + mock" >> "$LOG"
    uv run python scripts/generate_schema.py >> "$LOG" 2>&1

# Fast path: audit, doctor, injection, integration tests (no Docker, no cross-compile)
smoke: _install-tools _pnpm-install _check-assets _pack-initrd
    #!/bin/bash
    set -euo pipefail
    # Smoke runs against an isolated CAPSEM_HOME so it doesn't stomp on a
    # locally installed capsem daemon. _ensure-service is invoked below
    # (not as a just dep) so it inherits the exported env vars.
    export CAPSEM_HOME="{{justfile_directory()}}/target/test-home/.capsem"
    export CAPSEM_RUN_DIR="$CAPSEM_HOME/run"
    mkdir -p "$CAPSEM_RUN_DIR"
    LOCK_FILE="$CAPSEM_RUN_DIR/execution.lock"
    exec 3>"$LOCK_FILE"
    flock -n 3 || { echo "another agent holds the test execution lock ($LOCK_FILE); try again later"; exit 1; }
    just _ensure-service
    SMOKE_LOG="{{justfile_directory()}}/target/smoke.log"
    mkdir -p "$(dirname "$SMOKE_LOG")"
    exec > >(tee "$SMOKE_LOG") 2>&1
    SMOKE_START=$SECONDS
    step() { STEP_START=$SECONDS; echo "=== $1 ==="; }
    step_done() { echo "  -> $(( SECONDS - STEP_START ))s"; echo ""; }
    step "Rust check + audit (parallel)"
    cargo check --workspace &
    CHECK_PID=$!
    cargo audit &
    AUDIT_PID=$!
    (cd frontend && pnpm audit)
    wait $CHECK_PID || { echo "cargo check failed"; exit 1; }
    wait $AUDIT_PID || { echo "cargo audit failed"; exit 1; }
    step_done
    step "capsem-doctor --fast (in-VM diagnostics, no throughput)"
    {{cli_binary}} doctor --fast
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
    uv run python -m pytest tests/capsem-mcp/ -v --tb=short -m "mcp" &
    PID_MCP=$!
    uv run python -m pytest tests/capsem-service/ tests/capsem-cli/ -v --tb=short -m "integration" &
    PID_SVC=$!
    uv run python -m pytest tests/capsem-gateway/ -v --tb=short -m "gateway" &
    PID_GW=$!
    FAIL=0
    wait $PID_MCP || FAIL=1
    wait $PID_SVC || FAIL=1
    wait $PID_GW || FAIL=1
    [ $FAIL -eq 0 ] || { echo "Python tests failed"; exit 1; }
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
test-gateway-e2e: _check-assets _pack-initrd _sign
    #!/bin/bash
    set -euo pipefail
    LOCK_FILE="$HOME/.capsem/run/execution.lock"
    mkdir -p "$(dirname "$LOCK_FILE")"
    exec 3>"$LOCK_FILE"
    flock -n 3 || { echo "another agent holds the capsem execution lock ($LOCK_FILE); try again later"; exit 1; }
    cargo build -p capsem-gateway {{host_crates}}
    echo "=== Gateway: E2E tests (real service + VMs) ==="
    uv run python -m pytest tests/capsem-gateway/ -v --tb=short -m "gateway and e2e"

# Local HTML coverage report across all Rust crates
coverage:
    #!/bin/bash
    set -euo pipefail
    cargo llvm-cov --workspace --no-cfg-coverage --html
    echo "Coverage report: target/llvm-cov/html/index.html"
    open target/llvm-cov/html/index.html 2>/dev/null || true

# Run in-VM benchmarks (disk I/O, rootfs read, CLI startup, HTTP latency)
bench: _ensure-setup _check-assets _pack-initrd _ensure-service
    #!/bin/bash
    set -euo pipefail
    LOCK_FILE="$HOME/.capsem/run/execution.lock"
    mkdir -p "$(dirname "$LOCK_FILE")"
    exec 3>"$LOCK_FILE"
    flock -n 3 || { echo "another agent holds the capsem execution lock ($LOCK_FILE); try again later"; exit 1; }
    echo "=== In-VM benchmarks (disk, rootfs, CLI, HTTP, snapshots) ==="
    {{cli_binary}} run "capsem-bench"
    echo ""
    echo "=== Host-side benchmarks (lifecycle, fork) ==="
    uv run python -m pytest tests/capsem-serial/test_lifecycle_benchmark.py -v --tb=short -m serial

# Build the platform package (.pkg on macOS, .deb on Linux) and install it.
# Builds release binaries, frontend, and Tauri app. Asks for sudo to install.
# The postinstall script handles codesign, PATH, service registration, and setup.
install: _pnpm-install _stamp-version _check-assets _pack-initrd
    #!/bin/bash
    set -euo pipefail
    LOCK_FILE="$HOME/.capsem/run/execution.lock"
    mkdir -p "$(dirname "$LOCK_FILE")"
    exec 3>"$LOCK_FILE"
    flock -n 3 || { echo "another agent holds the capsem execution lock ($LOCK_FILE); try again later"; exit 1; }
    VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')
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
            "target/release/bundle/macos/Capsem.app" \
            "target/release" \
            "{{assets_dir}}" \
            "$VERSION"
        PKG="packages/Capsem-$VERSION.pkg"
        echo "=== Opening installer ==="
        open -W "$PKG"
    else
        echo "=== Building .deb ==="
        eval cargo tauri build --bundles deb $TAURI_FLAGS
        DEB=$(ls target/release/bundle/deb/*.deb)
        bash scripts/repack-deb.sh "$DEB" "target/release"
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
    echo "=== Syncing locally built assets into ~/.capsem/assets ==="
    bash scripts/sync-dev-assets.sh "{{assets_dir}}" "$HOME/.capsem/assets"
    echo "=== Pruning stale build artifacts ==="
    just _clean-stale

# Run install e2e tests in Docker (Linux + systemd).
# Builds the real .deb (Tauri + repack), installs with dpkg -i (exercises
# deb-postinst.sh), then runs the pytest suite against the installed layout.
test-install: _build-host
    #!/bin/bash
    set -euo pipefail
    IMAGE="capsem-install-test"
    # Build the Docker image if needed
    if ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
        echo "Building $IMAGE Docker image..."
        docker build -t "$IMAGE" -f docker/Dockerfile.install-test .
    fi
    CONTAINER="capsem-install-test-$$"
    echo "Starting systemd container..."
    docker run -d --name "$CONTAINER" \
        --privileged --cgroupns=host \
        -v /sys/fs/cgroup:/sys/fs/cgroup:rw \
        --tmpfs /run --tmpfs /tmp \
        -v "$PWD":/src \
        -v capsem-install-target:/cargo-target \
        -v capsem-install-cargo:/usr/local/cargo/registry \
        "$IMAGE" /usr/lib/systemd/systemd
    # Wait for systemd to be ready
    for i in $(seq 1 30); do
        if docker exec "$CONTAINER" systemctl is-system-running --wait 2>/dev/null | grep -qE "running|degraded"; then
            break
        fi
        sleep 0.5
    done
    # Fix ownership for capsem user builds
    docker exec "$CONTAINER" bash -c "mkdir -p /cargo-target && chown -R capsem:capsem /cargo-target /usr/local/cargo"
    echo "Building host binaries..."
    docker exec -u capsem "$CONTAINER" bash -c \
        "cd /src && cargo build {{host_crates}}"
    echo "Building frontend..."
    docker exec "$CONTAINER" bash -c "chown -R capsem:capsem /src/frontend/node_modules 2>/dev/null || true"
    docker exec -u capsem -e CI=true "$CONTAINER" bash -c \
        "cd /src/frontend && pnpm install && pnpm build"
    echo "Building Tauri .deb..."
    docker exec -u capsem "$CONTAINER" bash -c \
        "cd /src && cargo tauri build --debug --bundles deb --config '{\"bundle\":{\"createUpdaterArtifacts\":false}}'"
    echo "Repacking .deb with companion binaries..."
    docker exec -u capsem "$CONTAINER" bash -c \
        'cd /src && DEB=$(ls /cargo-target/debug/bundle/deb/*.deb) && bash scripts/repack-deb.sh "$DEB" /cargo-target/debug'
    echo "Installing .deb via dpkg..."
    docker exec "$CONTAINER" bash -c \
        "dpkg -i /cargo-target/debug/bundle/deb/*.deb 2>&1 || apt-get install -f -y"
    echo "Running install e2e tests..."
    docker exec -u capsem -e XDG_RUNTIME_DIR=/run/user/1000 -e CAPSEM_DEB_INSTALLED=1 "$CONTAINER" bash -c \
        "cd /src && uv run pytest tests/capsem-install/ -v --tb=short"
    EXIT_CODE=$?
    echo "Cleaning up container..."
    docker stop "$CONTAINER" >/dev/null 2>&1
    docker rm "$CONTAINER" >/dev/null 2>&1
    just _docker-gc
    exit $EXIT_CODE

# Wait for CI to build and publish a tag.
# Usage: just release          (uses latest vX.Y.Z tag on HEAD)
#        just release v0.9.13  (explicit tag)
release tag="":
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -n "{{tag}}" ]; then
        TAG="{{tag}}"
    else
        TAG=$(git tag --points-at HEAD 'v*' | sort -V | tail -1)
        if [ -z "$TAG" ]; then
            echo "Error: HEAD has no v* tag. Pass one explicitly: just release v0.9.13"
            exit 1
        fi
    fi
    echo "=== Release $TAG ==="
    RUN_ID=$(gh run list --workflow=release.yaml --json databaseId,headBranch,status \
        --jq ".[] | select(.headBranch==\"$TAG\") | .databaseId" | head -1)
    if [ -z "$RUN_ID" ]; then
        echo "Error: no release workflow run found for tag $TAG"
        echo "Push the tag first: git push origin $TAG"
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
    echo "=== Release $TAG published ==="
    echo "https://github.com/google/capsem/releases/tag/$TAG"

# Stamp version, commit, tag, push, and wait for CI to publish.
# Runs test first (all validation gates) to avoid burning tags on issues only CI would catch.
cut-release: test _stamp-version
    #!/usr/bin/env bash
    set -euo pipefail
    NEW=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')
    TAG="v${NEW}"
    TODAY=$(date +%Y-%m-%d)
    echo "=== Cutting release $TAG ==="
    # Stamp changelog: [Unreleased] -> [NEW] - TODAY
    sed -i '' "s/^## \[Unreleased\]/## [Unreleased]\n\n## [${NEW}] - ${TODAY}/" CHANGELOG.md
    # Extract latest release notes for the frontend boot screen
    uv run python3 scripts/extract-release-notes.py
    # Commit, tag, push
    git add Cargo.toml crates/capsem-app/tauri.conf.json pyproject.toml CHANGELOG.md LATEST_RELEASE.md
    git commit -m "release: v${NEW}"
    git tag "$TAG"
    git push origin main "$TAG"
    echo "Tag $TAG pushed. Waiting for CI..."
    just release "$TAG"

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
        # Find latest session that still has a session.db (skip vacuumed)
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
    curl -sL https://raw.githubusercontent.com/pydantic/genai-prices/main/prices/data_slim.json \
        -o config/genai-prices.json
    @echo "Updated config/genai-prices.json"

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
_ensure-setup:
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
        for f in vmlinuz initrd.img rootfs.squashfs; do
            [ -f "$dir/$arch/$f" ] || missing+=("$arch/$f")
        done
    elif [ -f "$dir/vmlinuz" ]; then
        # Flat layout (legacy): assets/vmlinuz
        for f in vmlinuz initrd.img; do
            [ -f "$dir/$f" ] || missing+=("$f")
        done
        [ -f "$dir/rootfs.squashfs" ] || missing+=("rootfs.squashfs")
    else
        missing+=("vmlinuz (checked $dir/$arch/ and $dir/)")
    fi
    if [ ${#missing[@]} -gt 0 ]; then
        echo "Missing VM assets in $dir/: ${missing[*]}"
        echo "Building assets (requires docker)..."
        just build-assets
    fi

_pnpm-install:
    cd frontend && pnpm install --frozen-lockfile

_frontend: _pnpm-install
    cd frontend && pnpm build

_compile: _frontend _clean-stale
    cargo build -p capsem

_sign-release: _compile
    #!/bin/bash
    set -euo pipefail
    if [[ "$(uname -s)" != "Darwin" ]]; then
        echo "  [skip] codesign (Linux -- not needed, using KVM)"
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
    for b in capsem-pty-agent capsem-net-proxy capsem-mcp-server capsem-sysutil; do
        if [ ! -f "$RELEASE_DIR/$b" ]; then
            NEED_BUILD=true
            break
        fi
    done
    # Also rebuild if any agent source is newer than the binaries
    if [ "$NEED_BUILD" = "false" ] && [ -f "$RELEASE_DIR/capsem-pty-agent" ]; then
        NEWEST_SRC=$(find "$ROOT/crates/capsem-agent" "$ROOT/crates/capsem-proto" -name '*.rs' -newer "$RELEASE_DIR/capsem-pty-agent" 2>/dev/null | head -1)
        if [ -n "$NEWEST_SRC" ]; then
            NEED_BUILD=true
        fi
    fi
    if [ "$NEED_BUILD" = "true" ]; then
        echo "=== Cross-compile agent ==="
        uv run capsem-builder agent --arch "$arch"
        echo ""
    else
        echo "=== Agent binaries up to date, skipping cross-compile ==="
    fi
    echo "=== Repack initrd ==="
    WORKDIR=$(mktemp -d)
    cd "$WORKDIR"
    gzip -dc "$INITRD" | cpio -id 2>/dev/null
    cp "$ROOT/guest/artifacts/capsem-init" init
    chmod 755 init
    # Verify binaries exist before repacking
    RELEASE_DIR="$ROOT/target/linux-agent/$arch"
    for b in capsem-pty-agent capsem-net-proxy capsem-mcp-server capsem-sysutil; do
        if [ ! -f "$RELEASE_DIR/$b" ]; then
            echo "ERROR: $b missing from $RELEASE_DIR"
            exit 1
        fi
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
    find . | cpio -o -H newc 2>/dev/null | gzip > "$INITRD"
    rm -rf "$WORKDIR"
    cd "$ROOT"
    # Regenerate checksums -- handle per-arch and flat layouts
    ASSETS="$ROOT/{{assets_dir}}"
    if [ -f "$ASSETS/$arch/vmlinuz" ]; then
        (cd "$ASSETS" && b3sum "$arch/vmlinuz" "$arch/initrd.img" "$arch/rootfs.squashfs" > B3SUMS)
    else
        (cd "$ASSETS" && b3sum vmlinuz initrd.img rootfs.squashfs > B3SUMS)
    fi
    # Generate manifest.json from B3SUMS + file sizes
    python3 "$ROOT/scripts/gen_manifest.py" "$ASSETS" "$ROOT/Cargo.toml"
    # Create hash-named copies so dev layout matches installed layout.
    python3 "$ROOT/scripts/create_hash_assets.py" "$ASSETS"
    # Force cargo to re-run build.rs so it picks up new manifest hashes
    touch "$ROOT/crates/capsem-app/build.rs"
    echo "initrd repacked (with agent + net-proxy + mcp-server + sysutil + doctor)"
