# Capsem Justfile
#
# Dependency chains:
#
#   doctor          read-only check of all required tools (user-facing)
#   _install-tools  auto-installs rust targets, components, cargo tools (internal)
#   _check-assets   verifies VM assets exist, tells you to run build-assets if not
#   audit           checks for known vulnerabilities in Rust + npm deps (gates all paths)
#
#   run             -> audit + _check-assets + _pack-initrd + _sign + _compile + _frontend
#   test            -> audit + _install-tools
#   build-assets    -> doctor + _install-tools + audit
#   test-injection  -> _check-assets + _pack-initrd + _sign
#   full-test       -> test + _check-assets + _pack-initrd + _sign
#   install         -> doctor + full-test
#
# First-time setup:
#   just doctor       (shows what's missing)
#   just build-assets (builds kernel, initrd, rootfs -- needs docker/podman)
#
# Daily dev:          just run     (fast ~10s, auto-repacks initrd)
# Before release:     just install (doctor + full-test -- all validation gates)
# Releases:           just cut-release (bump, tag, push, CI builds + publishes)
# Dep maintenance:    just update-deps (cargo update + pnpm update)

binary := "target/debug/capsem"
assets_dir := "assets"
entitlements := "entitlements.plist"

# Run the app in development mode with hot-reloading
dev: _pnpm-install
    @echo "Stopping running instances..."
    -@pkill -x capsem 2>/dev/null || true
    -@pkill -x Capsem 2>/dev/null || true
    CAPSEM_ASSETS_DIR={{assets_dir}} cargo tauri dev --config crates/capsem-app/tauri.conf.json

# Frontend-only dev server with mock data (no Tauri/VM needed)
ui: _pnpm-install
    cd frontend && pnpm run dev

# Pack + boot VM (interactive or with command, ~10s)
run *CMD: audit _check-assets _pack-initrd _sign
    #!/bin/bash
    set -euo pipefail
    pkill -x capsem 2>/dev/null || true
    CAPSEM_ASSETS_DIR={{assets_dir}} {{binary}} {{CMD}}

# Full VM asset rebuild (kernel, initrd, rootfs) via Docker/Podman
build-assets: doctor _install-tools audit
    cd images && python3 build.py

# Dependency audit: check for known vulnerabilities in Rust and npm deps
audit: _install-tools _pnpm-install
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

# Unit tests + cross-compile check + frontend type-check (no VM)
test: _install-tools _clean-stale audit _pnpm-install
    cargo llvm-cov --workspace --no-cfg-coverage
    cargo build --release --target aarch64-unknown-linux-musl -p capsem-agent 2>&1 | tail -3
    cd frontend && pnpm run check && pnpm run test && pnpm run build

# Full validation: test + capsem-doctor + injection test + integration test + bench (boots VM)
full-test: test _check-assets _pack-initrd _sign
    @echo ""
    @echo "=== capsem-doctor ==="
    CAPSEM_ASSETS_DIR={{assets_dir}} {{binary}} "capsem-doctor"
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
bench: _check-assets _sign
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
# Runs `just test` first (unit tests + audit + frontend check) to avoid burning tags.
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
    # Stamp changelog: [Unreleased] -> [NEW] - TODAY
    sed -i '' "s/^## \[Unreleased\]/## [Unreleased]\n\n## [${NEW}] - ${TODAY}/" CHANGELOG.md
    # Commit, tag, push
    git add Cargo.toml crates/capsem-app/tauri.conf.json CHANGELOG.md
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
    for tool in cargo rustup codesign pnpm node python3 sqlite3 git; do
        if command -v "$tool" &>/dev/null; then
            pass "$tool"
        else
            fail "$tool not found"
        fi
    done

    echo ""
    echo "== Container Runtime =="
    if command -v docker &>/dev/null; then
        pass "docker"
    elif command -v podman &>/dev/null; then
        pass "podman"
    else
        fail "docker or podman -- brew install podman && podman machine init && podman machine start"
    fi

    echo ""
    echo "== Rust Toolchain =="
    if rustup target list --installed 2>/dev/null | grep -q aarch64-unknown-linux-musl; then
        pass "target: aarch64-unknown-linux-musl"
    else
        fail "target: aarch64-unknown-linux-musl -- run: rustup target add aarch64-unknown-linux-musl"
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

# Clean build artifacts
clean:
    cargo clean
    cd frontend && rm -rf dist node_modules

# Deep clean: build artifacts + container images + podman cache
clean-all: clean
    #!/bin/bash
    set -euo pipefail
    # Remove stale rootfs files from target dirs
    find target -name "rootfs.img" -delete 2>/dev/null || true
    find target -name "rootfs.squashfs" -delete 2>/dev/null || true
    rm -rf target/llvm-cov-target
    # Prune container images
    if command -v podman &>/dev/null; then
        echo "Pruning podman images..."
        podman system prune -af
    elif command -v docker &>/dev/null; then
        echo "Pruning docker images..."
        docker system prune -af
    fi

# Inspect session DB integrity and event summary (latest by default)
inspect-session *args='':
    python3 scripts/check_session.py {{args}}

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

# Auto-install Rust targets, components, and cargo tools
_install-tools:
    #!/bin/bash
    set -euo pipefail
    # Musl target for cross-compiling guest binaries
    if ! rustup target list --installed | grep -q aarch64-unknown-linux-musl; then
        echo "Installing aarch64-unknown-linux-musl target..."
        rustup target add aarch64-unknown-linux-musl
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

# Verify VM assets exist (vmlinuz, initrd.img, rootfs)
_check-assets:
    #!/bin/bash
    set -euo pipefail
    dir="{{assets_dir}}"
    missing=()
    for f in vmlinuz initrd.img; do
        [ -f "$dir/$f" ] || missing+=("$f")
    done
    if [ ! -f "$dir/rootfs.squashfs" ]; then
        missing+=("rootfs.squashfs")
    fi
    if [ ${#missing[@]} -gt 0 ]; then
        echo "ERROR: Missing VM assets in $dir/: ${missing[*]}"
        echo ""
        echo "Run 'just build-assets' to build them (requires docker or podman)."
        exit 1
    fi

_pnpm-install:
    cd frontend && pnpm install --frozen-lockfile

_frontend: _pnpm-install
    cd frontend && pnpm build

_compile: _frontend _clean-stale
    cargo build -p capsem

_sign: _compile
    codesign --sign - --entitlements {{entitlements}} --force {{binary}}

_pack-initrd:
    #!/bin/bash
    set -euo pipefail
    ROOT="{{justfile_directory()}}"
    INITRD="$ROOT/{{assets_dir}}/initrd.img"
    if [ ! -f "$INITRD" ]; then
        echo "ERROR: $INITRD not found. Run 'just build-assets' first."
        exit 1
    fi
    echo "=== Cross-compile agent ==="
    cargo build --release --target aarch64-unknown-linux-musl -p capsem-agent 2>&1 | tail -3
    echo ""
    echo "=== Repack initrd ==="
    WORKDIR=$(mktemp -d)
    cd "$WORKDIR"
    gzip -dc "$INITRD" | cpio -id 2>/dev/null
    cp "$ROOT/images/capsem-init" init
    chmod 755 init
    rm -f capsem-pty-agent capsem-net-proxy capsem-mcp-server
    cp "$ROOT/target/aarch64-unknown-linux-musl/release/capsem-pty-agent" capsem-pty-agent
    chmod 555 capsem-pty-agent
    cp "$ROOT/target/aarch64-unknown-linux-musl/release/capsem-net-proxy" capsem-net-proxy
    chmod 555 capsem-net-proxy
    cp "$ROOT/target/aarch64-unknown-linux-musl/release/capsem-mcp-server" capsem-mcp-server
    chmod 555 capsem-mcp-server
    cp "$ROOT/images/capsem-doctor" capsem-doctor
    chmod 755 capsem-doctor
    cp "$ROOT/images/capsem-bench" capsem-bench
    chmod 755 capsem-bench
    rm -rf diagnostics
    cp -r "$ROOT/images/diagnostics" diagnostics
    find . | cpio -o -H newc 2>/dev/null | gzip > "$INITRD"
    rm -rf "$WORKDIR"
    cd "$ROOT"
    (cd "{{assets_dir}}" && b3sum vmlinuz initrd.img rootfs.squashfs > B3SUMS)
    # Generate manifest.json from B3SUMS + file sizes
    python3 "$ROOT/scripts/gen_manifest.py" "$ROOT/{{assets_dir}}" "$ROOT/Cargo.toml"
    # Force cargo to re-run build.rs so it picks up new manifest hashes
    touch "$ROOT/crates/capsem-app/build.rs"
    echo "initrd repacked (with agent + net-proxy + mcp-server + doctor)"
