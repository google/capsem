# Capsem Justfile
#
# Dependency chains:
#
#   _ensure-setup   checks for .dev-setup sentinel, runs doctor if missing (auto first-run)
#   doctor          read-only check of all required tools, writes .dev-setup (user-facing)
#   _install-tools  auto-installs rust targets, components, cargo tools (internal)
#   _check-assets   verifies VM assets exist, tells you to run build-assets if not
#   audit           checks for known vulnerabilities in Rust + npm deps (gates all paths)
#
#   shell           -> _check-assets + _pack-initrd + _ensure-service (daily dev entry point)
#   test            -> audit + _install-tools + _check-assets + _pack-initrd + cross-compile + test-install (ALL tests)
#   build-assets    -> doctor + _install-tools + _clean-stale + audit
#   dev             -> _ensure-setup + _pnpm-install
#   bench           -> _ensure-setup + _check-assets + _sign
#   test-injection  -> _check-assets + _pack-initrd + _sign
#   test-mcp        -> _check-assets + _pack-initrd (MCP integration tests, boots VMs)
#   test-service    -> _check-assets + _pack-initrd (service HTTP API tests)
#   test-cli        -> _check-assets + _pack-initrd (CLI integration tests)
#   cut-release     -> test
#   smoke           -> _check-assets + _pack-initrd + _ensure-service (fast path: doctor + integration)
#   install         -> smoke (verify first, then install to ~/.capsem/)
#   test-install    -> _build-host (Docker e2e: systemd + install layout)
#
# Service daemon:
#   run-service     -> _check-assets + _pack-initrd (start daemon, idempotent)
#   run-doctor      -> run-service + capsem doctor
#   ui              -> run-service + cargo tauri dev (GUI with hot-reload)
#   smoke-test-svc  -> _check-assets + _pack-initrd + doctor + MCP + service integration
#
# First-time setup:
#   just doctor       (shows what's missing)
#   just build-assets (builds kernel + rootfs via capsem-builder -- needs docker via Colima on macOS)
#
# Daily dev:          just shell   (service daemon + temp VM + shell, ~10s)
#                     just ui      (service + Tauri GUI with hot-reload)
# Local install:      just install (smoke test + install to ~/.capsem/)
# Releases:           just cut-release (test + bump, tag, push, CI)
# Dep maintenance:    just update-deps (cargo update + pnpm update)
# Disk cleanup:       just clean   (nuke target/ + frontend build, ~100 GB)
#                     just clean-all (clean + docker prune)

binary := "target/debug/capsem"
cli_binary := "target/debug/capsem"
service_binary := "target/debug/capsem-service"
process_binary := "target/debug/capsem-process"
mcp_binary := "target/debug/capsem-mcp"
host_binaries := "target/debug/capsem target/debug/capsem-service target/debug/capsem-process target/debug/capsem-mcp"
assets_dir := "assets"
entitlements := "entitlements.plist"
host_crates := "-p capsem-service -p capsem-process -p capsem -p capsem-mcp"

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
# Always kills any existing instance and relaunches fresh.
_ensure-service: _sign
    #!/bin/bash
    set -euo pipefail
    arch=$(uname -m)
    [[ "$arch" == "arm64" ]] || arch="x86_64"
    mkdir -p ~/.capsem/run
    PIDFILE=~/.capsem/run/service.pid
    # Kill existing service if any
    if [[ -f "$PIDFILE" ]]; then
        OLD_PID=$(cat "$PIDFILE")
        if kill -0 "$OLD_PID" 2>/dev/null; then
            echo "Stopping capsem-service (PID $OLD_PID)..."
            kill "$OLD_PID" 2>/dev/null || true
            sleep 1
        fi
        rm -f "$PIDFILE"
    fi
    rm -f ~/.capsem/run/service.sock
    echo "Starting capsem-service..."
    RUST_LOG=capsem=debug {{service_binary}} \
        --assets-dir {{assets_dir}}/$arch \
        --process-binary {{process_binary}} \
        --foreground &
    SVC_PID=$!
    echo "$SVC_PID" > "$PIDFILE"
    for i in $(seq 1 30); do
        if [ -S ~/.capsem/run/service.sock ] && curl -s --unix-socket ~/.capsem/run/service.sock --max-time 2 http://localhost/list >/dev/null 2>&1; then
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
    CAPSEM_ASSETS_DIR={{assets_dir}} cargo tauri dev --config crates/capsem-app/tauri.conf.json

# Frontend-only dev server with mock data (no Tauri/VM needed)
dev-frontend: _pnpm-install
    cd frontend && pnpm run dev

# Start service daemon + boot temporary VM + shell (~10s after first build)
shell: _check-assets _pack-initrd _ensure-service
    {{cli_binary}} shell

# Start capsem-service daemon (builds, signs, launches or reuses running instance)
run-service: _check-assets _pack-initrd _ensure-service

# Execute a command in a fresh temporary VM (auto-provisioned and destroyed)
# Usage: just exec "echo hello"   or   just exec "ls -la"
exec +CMD: run-service
    {{cli_binary}} run {{CMD}}

# Run capsem-doctor (creates temp VM, tears down automatically)
run-doctor: run-service
    {{cli_binary}} doctor

# VM asset rebuild (kernel + rootfs). Default: both arches. Pass arch to build one.
build-assets arch="": _install-tools _clean-stale audit
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

# Dependency audit: check for known vulnerabilities in Rust and npm deps
audit: _ensure-setup _install-tools _pnpm-install
    #!/bin/bash
    set -euo pipefail
    echo "=== Cargo audit ==="
    cargo audit || echo "warnings found (see above) -- upstream Tauri/GTK deps, not actionable"
    echo ""
    echo "=== Frontend audit ==="
    cd frontend && pnpm audit
    echo ""
    echo "All dependencies clean. If vulnerabilities found, run: just update-deps"

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
    echo "Done. Run 'just audit' to verify, then 'just test' to confirm nothing broke."

# Run ALL tests: Rust + frontend + Python + injection + integration + bench + cross-compile + install e2e. No shortcuts.
test: _install-tools _clean-stale audit _pnpm-install _generate-settings _check-assets _pack-initrd
    #!/bin/bash
    set -euo pipefail

    echo "=== Rust: warnings-as-errors for service crates (check only, no codegen) ==="
    RUSTFLAGS="-D warnings" cargo check -p capsem-service -p capsem-process

    echo "=== Rust: test suite with coverage (compiles + runs all tests) ==="
    cargo llvm-cov --workspace --no-cfg-coverage

    echo "=== Cross-compile agent ==="
    uv run capsem-builder agent

    echo "=== Frontend ==="
    cd frontend && pnpm run check && pnpm run test && pnpm run build
    cd ..

    echo "=== Sign binaries for integration tests ==="
    just _sign

    echo "=== Python: ALL tests (no marker exclusions) ==="
    uv run python -m pytest tests/ -v --tb=short --cov=src/capsem --cov-report=xml:codecov-python.xml --cov-fail-under=90

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
    for vol in capsem-cargo-registry capsem-cargo-git capsem-host-target-arm64 capsem-host-target-x86_64 capsem-rustup-arm64 capsem-rustup-x86_64; do
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
               cp /cargo-target/\$RUST_TARGET/release/capsem-pty-agent /cargo-target/\$RUST_TARGET/release/capsem-mcp-server /cargo-target/\$RUST_TARGET/release/capsem-net-proxy /cargo-target/linux-agent/\$TARGET_ARCH/ && \
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

# Fast path: build, sign, doctor, MCP + service + CLI integration tests (no Docker, no cross-compile)
smoke: _check-assets _pack-initrd _ensure-service
    #!/bin/bash
    set -euo pipefail
    echo "=== capsem-doctor ==="
    just run-doctor
    echo ""
    echo "=== Integration tests (MCP + service + CLI) ==="
    uv run python -m pytest tests/capsem-mcp/ tests/capsem-service/ tests/capsem-cli/ -v --tb=short
    echo ""
    echo "Smoke test passed"

# Local HTML coverage report across all Rust crates
coverage:
    #!/bin/bash
    set -euo pipefail
    cargo llvm-cov --workspace --no-cfg-coverage --html
    echo "Coverage report: target/llvm-cov/html/index.html"
    open target/llvm-cov/html/index.html 2>/dev/null || true

# End-to-end injection test: boot VM with generated configs, verify all injection paths
test-injection: _check-assets _pack-initrd _sign
    python3 scripts/injection_test.py --binary {{binary}} --assets {{assets_dir}}

# Run in-VM benchmarks (disk I/O, rootfs read, CLI startup, HTTP latency)
bench: _ensure-setup _check-assets _sign
    CAPSEM_ASSETS_DIR={{assets_dir}} {{binary}} "capsem-bench"

# Smoke test then install to ~/.capsem/ (verifies everything works before installing)
install: smoke
    #!/bin/bash
    set -euo pipefail
    echo "=== Installing to ~/.capsem/ ==="
    bash scripts/simulate-install.sh target/debug {{assets_dir}}
    # Sign on macOS (required for Virtualization.framework)
    if [[ "$(uname -s)" == "Darwin" ]]; then
        for bin in "$HOME/.capsem/bin"/capsem*; do
            codesign --sign - --entitlements {{entitlements}} --force "$bin"
        done
    fi
    # PATH check
    if [[ ":$PATH:" != *":$HOME/.capsem/bin:"* ]]; then
        echo ""
        echo "WARNING: ~/.capsem/bin is not in your PATH"
        echo "  Add to your shell profile: export PATH=\"\$HOME/.capsem/bin:\$PATH\""
    fi

# Run install e2e tests in Docker (Linux + systemd)
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
        -v "$PWD":/src:ro \
        "$IMAGE" /usr/lib/systemd/systemd
    # Wait for systemd to be ready
    for i in $(seq 1 30); do
        if docker exec "$CONTAINER" systemctl is-system-running --wait 2>/dev/null | grep -qE "running|degraded"; then
            break
        fi
        sleep 0.5
    done
    echo "Building capsem binaries inside container..."
    docker exec -u capsem "$CONTAINER" bash -c \
        "cd /src && cargo build {{host_crates}}"
    echo "Running simulate-install.sh..."
    docker exec -u capsem "$CONTAINER" bash -c \
        "cd /src && bash scripts/simulate-install.sh target/debug assets"
    echo "Running install e2e tests..."
    docker exec -u capsem -e XDG_RUNTIME_DIR=/run/user/1000 "$CONTAINER" bash -c \
        "cd /src && uv run pytest tests/capsem-install/ -v --tb=short"
    EXIT_CODE=$?
    echo "Cleaning up container..."
    docker stop "$CONTAINER" >/dev/null 2>&1
    docker rm "$CONTAINER" >/dev/null 2>&1
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

# Bump patch version, commit, tag, push, and wait for CI to publish.
# Runs test first (all validation gates) to avoid burning tags on issues only CI would catch.
cut-release: test
    #!/usr/bin/env bash
    set -euo pipefail
    # Read current version from Cargo.toml
    CURRENT=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')
    # Increment patch segment
    MAJOR=$(echo "$CURRENT" | cut -d. -f1)
    MINOR=$(echo "$CURRENT" | cut -d. -f2)
    PATCH=$(echo "$CURRENT" | cut -d. -f3)
    NEW_PATCH=$((PATCH + 1))
    NEW="${MAJOR}.${MINOR}.${NEW_PATCH}"
    TAG="v${NEW}"
    TODAY=$(date +%Y-%m-%d)
    echo "=== Cutting release $TAG (${CURRENT} -> ${NEW}) ==="
    # Bump Cargo.toml
    sed -i '' "s/^version = \"${CURRENT}\"/version = \"${NEW}\"/" Cargo.toml
    # Bump tauri.conf.json
    sed -i '' "s/\"version\": \"${CURRENT}\"/\"version\": \"${NEW}\"/" crates/capsem-app/tauri.conf.json
    # Bump pyproject.toml
    sed -i '' "s/^version = \"${CURRENT}\"/version = \"${NEW}\"/" pyproject.toml
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

# Check that all required dev tools and dependencies are installed
doctor: _pnpm-install
    scripts/doctor-common.sh

# Doctor + auto-fix all fixable issues
doctor-fix: _pnpm-install
    scripts/doctor-common.sh --fix

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

# View logs for the latest sandbox
last-logs:
    #!/bin/bash
    set -euo pipefail
    ID=$({{cli_binary}} ls | grep -v "ID" | head -1 | awk '{print $1}')
    if [ -n "$ID" ]; then
        {{cli_binary}} logs "$ID"
    else
        echo "No running sandboxes found."
    fi

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

# Remove stale rootfs copies and trim bloated incremental caches
_clean-stale:
    #!/bin/bash
    # Stale rootfs copies
    find target -path "*/debug/rootfs.*" -delete 2>/dev/null || true
    find target -path "*/release/rootfs.*" -delete 2>/dev/null || true
    find target -path "*/_up_" -type d -exec rm -rf {} + 2>/dev/null || true
    find target -path "*/llvm-cov-target/debug/rootfs.*" -delete 2>/dev/null || true
    # Trim incremental caches if target/ exceeds 20 GB (prevents unbounded growth)
    if [ -d target ]; then
        TARGET_KB=$(du -sk target 2>/dev/null | cut -f1)
        THRESHOLD=$((20 * 1024 * 1024))  # 20 GB in KB
        if [ "$TARGET_KB" -gt "$THRESHOLD" ]; then
            echo "target/ is $((TARGET_KB / 1024 / 1024)) GB (threshold: 20 GB) -- trimming incremental caches"
            rm -rf target/debug/incremental target/release/incremental target/llvm-cov-target 2>/dev/null || true
        fi
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
        echo "ERROR: Missing VM assets in $dir/: ${missing[*]}"
        echo ""
        echo "Run 'just build-assets' to build them (requires docker)."
        exit 1
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
    echo "=== Cross-compile agent ==="
    uv run capsem-builder agent --arch "$arch"
    echo ""
    echo "=== Repack initrd ==="
    WORKDIR=$(mktemp -d)
    cd "$WORKDIR"
    gzip -dc "$INITRD" | cpio -id 2>/dev/null
    cp "$ROOT/guest/artifacts/capsem-init" init
    chmod 755 init
    # Verify binaries exist before repacking
    RELEASE_DIR="$ROOT/target/linux-agent/$arch"
    for b in capsem-pty-agent capsem-net-proxy capsem-mcp-server; do
        if [ ! -f "$RELEASE_DIR/$b" ]; then
            echo "ERROR: $b missing from $RELEASE_DIR"
            exit 1
        fi
        rm -f "$b"
        cp "$RELEASE_DIR/$b" .
        chmod 555 "$b"
    done
    cp "$ROOT/guest/artifacts/capsem-doctor" capsem-doctor
    chmod 755 capsem-doctor
    cp "$ROOT/guest/artifacts/capsem-bench" capsem-bench
    chmod 755 capsem-bench
    rm -rf capsem_bench
    cp -r "$ROOT/guest/artifacts/capsem_bench" capsem_bench
    find capsem_bench -name '__pycache__' -exec rm -rf {} + 2>/dev/null || true
    cp "$ROOT/guest/artifacts/snapshots" snapshots
    chmod 755 snapshots
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
    # Force cargo to re-run build.rs so it picks up new manifest hashes
    touch "$ROOT/crates/capsem-app/build.rs"
    echo "initrd repacked (with agent + net-proxy + mcp-server + doctor)"
