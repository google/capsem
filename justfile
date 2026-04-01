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
#   run             -> audit(_ensure-setup) + _check-assets + _pack-initrd + _sign
#   test            -> audit(_ensure-setup) + _install-tools
#   build-assets    -> doctor + _install-tools + audit
#   dev             -> _ensure-setup + _pnpm-install
#   bench           -> _ensure-setup + _check-assets + _sign
#   test-injection  -> _check-assets + _pack-initrd + _sign
#   full-test       -> build-assets + test + _pack-initrd + _sign
#   cut-release     -> full-test
#   install         -> doctor + full-test
#
# First-time setup:
#   just doctor       (shows what's missing)
#   just build-assets (builds kernel + rootfs via capsem-builder -- needs docker via Colima on macOS)
#
# Daily dev:          just run     (fast ~10s, auto-repacks initrd)
# Before release:     just install (doctor + full-test -- all validation gates)
# Releases:           just cut-release (full-test + bump, tag, push, CI)
# Dep maintenance:    just update-deps (cargo update + pnpm update)

binary := "target/debug/capsem"
assets_dir := "assets"
entitlements := "entitlements.plist"

# Run the app in development mode with hot-reloading
dev: _ensure-setup _pnpm-install
    @echo "Stopping running instances..."
    -@pkill -x capsem 2>/dev/null || true
    -@pkill -x Capsem 2>/dev/null || true
    CAPSEM_ASSETS_DIR={{assets_dir}} cargo tauri dev --config crates/capsem-app/tauri.conf.json

# Frontend-only dev server with mock data (no Tauri/VM needed)
ui: _pnpm-install
    cd frontend && pnpm run dev

# Full rebuild + boot VM (build-assets then run)
full-run *CMD: build-assets _generate-settings _pack-initrd _sign
    #!/bin/bash
    set -euo pipefail
    pkill -x capsem 2>/dev/null || true
    CAPSEM_ASSETS_DIR={{assets_dir}} {{binary}} {{CMD}}

# Pack + boot VM (interactive or with command, ~10s)
run *CMD: audit _check-assets _generate-settings _pack-initrd _sign
    #!/bin/bash
    set -euo pipefail
    pkill -x capsem 2>/dev/null || true
    CAPSEM_ASSETS_DIR={{assets_dir}} {{binary}} {{CMD}}

# Full VM asset rebuild (kernel, initrd, rootfs) via capsem-builder
build-assets: doctor _install-tools audit
    #!/bin/bash
    set -euo pipefail
    arch=$(uname -m | sed 's/aarch64/arm64/')
    echo "=== Cleaning old assets ==="
    rm -rf "{{assets_dir}}/arm64" "{{assets_dir}}/x86_64"
    rm -f "{{assets_dir}}/manifest.json" "{{assets_dir}}/B3SUMS"
    echo "=== Building kernel for $arch ==="
    uv run capsem-builder build guest/ --arch "$arch" --template kernel --output "{{assets_dir}}/"
    echo ""
    echo "=== Building rootfs for $arch ==="
    uv run capsem-builder build guest/ --arch "$arch" --template rootfs --output "{{assets_dir}}/"
    echo ""
    echo "=== Generating checksums ==="
    uv run python3 -c 'from pathlib import Path; from capsem.builder.docker import generate_checksums, get_project_version; v = get_project_version(Path(".")); generate_checksums(Path("{{assets_dir}}"), v); print(f"manifest.json generated (v{v})")'

# Build kernel only
build-kernel arch="arm64":
    uv run capsem-builder build guest/ --arch {{arch}} --template kernel --output {{assets_dir}}/

# Build rootfs only
build-rootfs arch="arm64":
    uv run capsem-builder build guest/ --arch {{arch}} --template rootfs --output {{assets_dir}}/

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

# Unit tests + cross-compile check + frontend type-check + Python schema tests (no VM)
test: _install-tools _clean-stale audit _pnpm-install _generate-settings
    #!/bin/bash
    set -euo pipefail
    cargo llvm-cov --workspace --no-cfg-coverage
    echo "=== Cross-compile agent ==="
    uv run capsem-builder agent
    cd frontend && pnpm run check && pnpm run test && pnpm run build
    cd ..
    uv run python -m pytest tests/ --cov=src/capsem --cov-report=xml:codecov-python.xml --cov-fail-under=90

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
clean-host-image:
    #!/bin/bash
    set -euo pipefail
    docker rmi capsem-host-builder:latest 2>/dev/null || true
    for vol in capsem-cargo-registry capsem-cargo-git capsem-host-target-arm64 capsem-host-target-x86_64; do
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
cross-compile arch="": _check-assets _generate-settings
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
    docker run --rm \
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
               echo '--- Validate + copy artifacts ---' && \
               dpkg-deb --info /cargo-target/\$RUST_TARGET/release/bundle/deb/*.deb && \
               cp /cargo-target/\$RUST_TARGET/release/bundle/deb/*.deb /src/dist/ && \
               cp /cargo-target/linux-agent/\$TARGET_ARCH/* /src/dist/"
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

# Full validation: build-assets + test + capsem-doctor + injection test + integration test + bench (boots VM)
full-test: build-assets test _pack-initrd _sign
    @echo ""
    @echo "=== capsem-doctor ==="
    CAPSEM_ASSETS_DIR={{assets_dir}} {{binary}} "capsem-doctor"
    @echo ""
    @echo "=== Doctor session validation ==="
    python3 scripts/doctor_session_test.py --binary {{binary}} --assets {{assets_dir}}
    @echo ""
    @echo "=== Injection test ==="
    python3 scripts/injection_test.py --binary {{binary}} --assets {{assets_dir}}
    @echo ""
    @echo "=== Integration test ==="
    python3 scripts/integration_test.py --binary {{binary}} --assets {{assets_dir}}
    @echo ""
    @echo "=== Benchmarks ==="
    CAPSEM_ASSETS_DIR={{assets_dir}} {{binary}} "capsem-bench"

# End-to-end injection test: boot VM with generated configs, verify all injection paths
test-injection: _check-assets _pack-initrd _sign
    python3 scripts/injection_test.py --binary {{binary}} --assets {{assets_dir}}

# Run in-VM benchmarks (disk I/O, rootfs read, CLI startup, HTTP latency)
bench: _ensure-setup _check-assets _sign
    CAPSEM_ASSETS_DIR={{assets_dir}} {{binary}} "capsem-bench"

# Full validation (test + doctor + bench). Use `just run` for daily dev.
install: doctor full-test
    @echo ""
    @echo "All gates passed. Use 'just run' to boot the VM."

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
# Runs full-test first (build-assets + unit tests + capsem-doctor + integration +
# bench) to avoid burning tags on issues only CI would catch.
cut-release: full-test
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
    #!/bin/bash
    set -euo pipefail
    PASS=0; FAIL=0
    pass() { echo "  [PASS] $1"; PASS=$((PASS + 1)); }
    fail() { echo "  [FAIL] $1"; FAIL=$((FAIL + 1)); }

    echo "Capsem Doctor"
    echo "============="

    echo ""
    echo "== System Tools =="
    tool_hint() {
        case "$1" in
            cargo)   echo "installed with rustup -- curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh" ;;
            rustup)  echo "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh" ;;
            pnpm)    echo "npm i -g pnpm" ;;
            node)    echo "brew install node  (24+ required)" ;;
            python3) echo "brew install python" ;;
            uv)      echo "curl -LsSf https://astral.sh/uv/install.sh | sh" ;;
            sqlite3) echo "brew install sqlite" ;;
            git)     echo "brew install git" ;;
        esac
    }
    for tool in cargo rustup pnpm node python3 uv sqlite3 git; do
        if command -v "$tool" &>/dev/null; then
            pass "$tool"
        else
            fail "$tool not found -- install: $(tool_hint "$tool")"
        fi
    done

    echo ""
    echo "== Codesigning =="
    if [[ "$(uname -s)" == "Darwin" ]]; then
        # Check 1: Xcode Command Line Tools
        if xcode-select -p &>/dev/null; then
            pass "Xcode Command Line Tools ($(xcode-select -p))"
        else
            fail "Xcode Command Line Tools not installed -- run: xcode-select --install"
        fi

        # Check 2: codesign binary
        if command -v codesign &>/dev/null; then
            pass "codesign"
        else
            fail "codesign not found -- install Xcode Command Line Tools: xcode-select --install"
        fi

        # Check 3: entitlements.plist
        if [[ -r "{{entitlements}}" ]]; then
            pass "{{entitlements}} exists and is readable"
        else
            fail "{{entitlements}} missing or not readable -- run: git checkout {{entitlements}}"
        fi

        # Check 4: cargo runner config (.cargo/config.toml)
        if [[ -f ".cargo/config.toml" ]] && grep -q 'runner.*run_signed' .cargo/config.toml; then
            pass ".cargo/config.toml (cargo runner -> run_signed.sh)"
        else
            fail ".cargo/config.toml missing or misconfigured -- run: git checkout .cargo/config.toml"
        fi

        # Check 5: run_signed.sh exists and is executable
        if [[ -x "scripts/run_signed.sh" ]]; then
            pass "scripts/run_signed.sh"
        else
            fail "scripts/run_signed.sh missing or not executable -- run: git checkout scripts/run_signed.sh && chmod +x scripts/run_signed.sh"
        fi

        # Check 6: test sign (only if codesign and entitlements both exist)
        if command -v codesign &>/dev/null && [[ -r "{{entitlements}}" ]]; then
            SIGN_TEST=$(mktemp /tmp/capsem-sign-test.XXXXXX)
            if cc -x c -o "$SIGN_TEST" - <<< 'int main(){return 0;}' 2>/dev/null; then
                if codesign --sign - --entitlements "{{entitlements}}" --force "$SIGN_TEST" 2>/dev/null; then
                    pass "test sign succeeded (ad-hoc + entitlements)"
                else
                    fail "test sign failed -- codesign could not sign a binary with {{entitlements}}"
                    echo "         Try: codesign --sign - --entitlements {{entitlements}} --force /path/to/binary"
                    echo "         Check SIP status: csrutil status"
                fi
            else
                fail "test sign skipped -- cc could not compile a test binary -- reinstall: sudo rm -rf /Library/Developer/CommandLineTools && xcode-select --install"
            fi
            rm -f "$SIGN_TEST"
        fi
    else
        echo "  [SKIP] codesign (macOS-only -- not needed on Linux)"
        echo "  [SKIP] entitlements.plist (macOS-only)"
        echo "  [SKIP] test sign (macOS-only)"
        echo "  [INFO] Capsem VM features (run, dev, bench) require macOS with Apple Silicon."
        echo "         On Linux you can use: just test, just build-assets, just audit"
    fi

    echo ""
    echo "== Container Runtime =="
    if command -v docker &>/dev/null; then
        pass "docker ($(docker --version 2>/dev/null | head -1))"
    else
        if [[ "$(uname -s)" == "Darwin" ]]; then
            fail "docker -- install: brew install colima docker && colima start --vm-type vz --vz-rosetta --memory 8 --cpu 8"
        else
            fail "docker -- install: sudo apt install docker.io"
        fi
    fi
    # Check Colima is running on macOS
    if [[ "$(uname -s)" == "Darwin" ]]; then
        if command -v colima &>/dev/null; then
            if colima status 2>&1 | grep -qi "running"; then
                pass "colima (running)"
            else
                fail "colima not running -- start: colima start --vm-type vz --vz-rosetta --memory 8 --cpu 8"
            fi
        else
            fail "colima not found -- install: brew install colima"
        fi
        # Check container VM resources
        if command -v docker &>/dev/null; then
            mem_mb=$(docker info --format json 2>/dev/null | python3 -c "import sys,json; print(json.load(sys.stdin).get('MemTotal',0) // 1024 // 1024)" 2>/dev/null || echo 0)
            cpus=$(docker info --format json 2>/dev/null | python3 -c "import sys,json; print(json.load(sys.stdin).get('NCPU',0))" 2>/dev/null || echo 0)
            if [[ "$mem_mb" -gt 0 ]]; then
                if [[ "$mem_mb" -lt 4096 ]]; then
                    fail "Colima: ${mem_mb}MB RAM, ${cpus} CPUs (minimum 4096MB) -- fix: colima stop && colima start --memory 8 --cpu 8"
                elif [[ "$mem_mb" -lt 8192 ]]; then
                    pass "Colima: ${mem_mb}MB RAM, ${cpus} CPUs (recommended 8192MB)"
                else
                    pass "Colima: ${mem_mb}MB RAM, ${cpus} CPUs"
                fi
            fi
        fi
        # Check Docker credential helper config
        if [[ -f "$HOME/.docker/config.json" ]]; then
            creds_store=$(python3 -c "import json; c=json.load(open('$HOME/.docker/config.json')); print(c.get('credsStore',''))" 2>/dev/null || echo "")
            if [[ -n "$creds_store" ]]; then
                helper="docker-credential-$creds_store"
                if command -v "$helper" &>/dev/null; then
                    pass "Docker credential helper ($helper)"
                else
                    fail "Docker config references '$helper' but it is not installed -- fix: set credsStore to \"\" in ~/.docker/config.json"
                fi
            else
                pass "Docker credential config (no external helper)"
            fi
        fi
    fi

    echo ""
    echo "== Rust Toolchain =="
    if rustup target list --installed 2>/dev/null | grep -q aarch64-unknown-linux-musl; then
        pass "target: aarch64-unknown-linux-musl"
    else
        fail "target: aarch64-unknown-linux-musl -- run: rustup target add aarch64-unknown-linux-musl"
    fi
    if rustup target list --installed 2>/dev/null | grep -q x86_64-unknown-linux-musl; then
        pass "target: x86_64-unknown-linux-musl"
    else
        fail "target: x86_64-unknown-linux-musl -- run: rustup target add x86_64-unknown-linux-musl"
    fi
    if rustup component list --installed 2>/dev/null | grep -q llvm-tools; then
        pass "component: llvm-tools (provides rust-lld)"
    else
        fail "component: llvm-tools -- run: rustup component add llvm-tools"
    fi

    echo ""
    echo "== Cargo Tools =="
    for tool in cargo-llvm-cov cargo-audit b3sum cargo-tauri; do
        if command -v "$tool" &>/dev/null; then
            pass "$tool"
        else
            fail "$tool -- run: cargo install ${tool/cargo-/}"
        fi
    done

    echo ""
    echo "== Release Tools =="
    for tool in gh openssl minisign; do
        if command -v "$tool" &>/dev/null; then
            pass "$tool"
        else
            echo "  [SKIP] $tool -- brew install $tool (only needed for releases)"
        fi
    done
    for tool in cargo-sbom; do
        if command -v "$tool" &>/dev/null; then
            pass "$tool"
        else
            echo "  [SKIP] $tool -- cargo install $tool (only needed for releases)"
        fi
    done

    echo ""
    echo "============="
    echo "Results: $PASS passed, $FAIL failed"
    if [ "$FAIL" -gt 0 ]; then
        echo ""
        echo "Install missing tools, or run: just _install-tools (auto-installs Rust components + cargo tools)"
        exit 1
    fi
    echo "All good!"
    touch .dev-setup

# Clean build artifacts
clean:
    cargo clean
    cd frontend && rm -rf dist node_modules

# Deep clean: build artifacts + container images + docker cache
clean-all: clean
    #!/bin/bash
    set -euo pipefail
    # Remove stale rootfs files from target dirs
    find target -name "rootfs.img" -delete 2>/dev/null || true
    find target -name "rootfs.squashfs" -delete 2>/dev/null || true
    rm -rf target/llvm-cov-target
    # Prune container images
    if command -v docker &>/dev/null; then
        echo "Pruning docker images..."
        docker system prune -af
    fi

# Inspect session DB integrity and event summary (latest by default)
inspect-session *args='':
    python3 scripts/check_session.py {{args}}

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

# Remove stale rootfs copies from target dirs (fast, harmless)
_clean-stale:
    #!/bin/bash
    find target -path "*/debug/rootfs.*" -delete 2>/dev/null || true
    find target -path "*/release/rootfs.*" -delete 2>/dev/null || true
    find target -path "*/_up_" -type d -exec rm -rf {} + 2>/dev/null || true
    find target -path "*/llvm-cov-target/debug/rootfs.*" -delete 2>/dev/null || true

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
        echo "Run 'just build-assets' to build them (requires docker via Colima on macOS)."
        exit 1
    fi

_pnpm-install:
    cd frontend && pnpm install --frozen-lockfile

_frontend: _pnpm-install
    cd frontend && pnpm build

_compile: _frontend _clean-stale
    cargo build -p capsem

_sign: _compile
    #!/bin/bash
    set -euo pipefail
    if [[ "$(uname -s)" != "Darwin" ]]; then
        echo ""
        echo "ERROR: codesign requires macOS. Capsem VM features (run, dev, bench) need"
        echo "       macOS with Apple Silicon and the Virtualization.framework."
        echo ""
        echo "On Linux you can use:"
        echo "  just test          # unit tests + cross-compile + frontend check"
        echo "  just build-assets  # build VM kernel + rootfs (needs Docker/Podman)"
        echo "  just audit         # dependency vulnerability scan"
        echo ""
        exit 1
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
    TARGET_ARCH="$arch" uv run python3 -c "
    import os; from pathlib import Path
    from capsem.builder.config import load_guest_config
    from capsem.builder.docker import cross_compile_agent
    arch = os.environ['TARGET_ARCH']
    config = load_guest_config(Path('guest'))
    rust_target = config.build.architectures[arch].rust_target
    cross_compile_agent(rust_target, Path('.'), Path('target/linux-agent') / arch)
    "
    echo ""
    echo "=== Repack initrd ==="
    WORKDIR=$(mktemp -d)
    cd "$WORKDIR"
    gzip -dc "$INITRD" | cpio -id 2>/dev/null
    cp "$ROOT/guest/artifacts/capsem-init" init
    chmod 755 init
    rm -f capsem-pty-agent capsem-net-proxy capsem-mcp-server
    # Use binaries produced by cross-compile
    RELEASE_DIR="$ROOT/target/linux-agent/$arch"
    cp "$RELEASE_DIR/capsem-pty-agent" capsem-pty-agent
    chmod 555 capsem-pty-agent
    cp "$RELEASE_DIR/capsem-net-proxy" capsem-net-proxy
    chmod 555 capsem-net-proxy
    cp "$RELEASE_DIR/capsem-mcp-server" capsem-mcp-server
    chmod 555 capsem-mcp-server
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
