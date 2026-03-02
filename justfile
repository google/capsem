# Capsem Justfile

binary := "target/debug/capsem"
release_app := "target/release/bundle/macos/Capsem.app"
assets_dir := "assets"
entitlements := "entitlements.plist"

# Run the app in development mode with hot-reloading
dev:
    @echo "Stopping running instances..."
    -@pkill -x capsem 2>/dev/null || true
    -@pkill -x Capsem 2>/dev/null || true
    cargo tauri dev --config crates/capsem-app/tauri.conf.json

# Frontend-only dev server with mock data (no Tauri/VM needed)
ui:
    cd frontend && pnpm run dev

# Pack + boot VM (interactive or with command, ~10s)
run *CMD: _pack-initrd _sign
    CAPSEM_ASSETS_DIR={{assets_dir}} {{binary}} {{CMD}}

# Full VM asset rebuild (kernel, initrd, rootfs) via Docker/Podman
build-assets: _ensure-tools test
    cd images && python3 build.py

# Unit tests + cross-compile check + frontend type-check (no VM)
test: _ensure-tools
    cargo llvm-cov --workspace --no-cfg-coverage
    cargo build --release --target aarch64-unknown-linux-musl -p capsem-agent 2>&1 | tail -3
    cd frontend && pnpm run check && pnpm run build

# Full validation: test + capsem-doctor + integration test + bench (boots VM)
full-test: test _sign
    @echo ""
    @echo "=== capsem-doctor ==="
    CAPSEM_ASSETS_DIR={{assets_dir}} {{binary}} "capsem-doctor"
    @echo ""
    @echo "=== Integration test ==="
    python3 scripts/integration_test.py --binary {{binary}} --assets {{assets_dir}}
    @echo ""
    @echo "=== Benchmarks ==="
    CAPSEM_ASSETS_DIR={{assets_dir}} {{binary}} "capsem-bench"

# Run in-VM benchmarks (disk I/O, rootfs read, CLI startup, HTTP latency)
bench: _sign
    CAPSEM_ASSETS_DIR={{assets_dir}} {{binary}} "capsem-bench"

# Build release .app + codesign + produce DMG
release: full-test _frontend
    cd crates/capsem-app && cargo tauri build
    codesign --sign - --entitlements {{entitlements}} --force --deep "{{release_app}}"
    @echo ""
    @echo "=== Create DMG ==="
    rm -f target/release/Capsem.dmg
    hdiutil create -volname Capsem -srcfolder "{{release_app}}" \
        -ov -format UDZO target/release/Capsem.dmg
    @echo "DMG: target/release/Capsem.dmg"

# Build release .app + install to /Applications + launch
install: full-test _frontend
    cd crates/capsem-app && cargo tauri build
    codesign --sign - --entitlements {{entitlements}} --force --deep "{{release_app}}"
    @echo "Stopping running Capsem..."
    -@pkill -x Capsem 2>/dev/null || true
    -@pkill -x capsem 2>/dev/null || true
    @echo "Installing to /Applications..."
    rm -rf "/Applications/Capsem.app"
    cp -R "{{release_app}}" "/Applications/"
    @echo "Launching Capsem..."
    open "/Applications/Capsem.app"

# Clean build artifacts
clean:
    cargo clean
    cd frontend && rm -rf dist node_modules
    rm -rf target/release/bundle/macos/Capsem.app target/release/Capsem.dmg

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

# --- Internal helpers (hidden from `just --list`) ---

_ensure-tools:
    #!/bin/bash
    set -euo pipefail
    if ! command -v cargo-llvm-cov &>/dev/null; then
        echo "Installing cargo-llvm-cov..."
        cargo install cargo-llvm-cov
    fi
    if ! rustup component list --installed | grep -q llvm-tools; then
        echo "Installing llvm-tools-preview..."
        rustup component add llvm-tools-preview
    fi

_frontend:
    cd frontend && pnpm build

_compile: _frontend
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
    rm -f capsem-pty-agent capsem-net-proxy capsem-mcp-server capsem-fs-watch
    cp "$ROOT/target/aarch64-unknown-linux-musl/release/capsem-pty-agent" capsem-pty-agent
    chmod 555 capsem-pty-agent
    cp "$ROOT/target/aarch64-unknown-linux-musl/release/capsem-net-proxy" capsem-net-proxy
    chmod 555 capsem-net-proxy
    cp "$ROOT/target/aarch64-unknown-linux-musl/release/capsem-mcp-server" capsem-mcp-server
    chmod 555 capsem-mcp-server
    cp "$ROOT/target/aarch64-unknown-linux-musl/release/capsem-fs-watch" capsem-fs-watch
    chmod 555 capsem-fs-watch
    find . | cpio -o -H newc 2>/dev/null | gzip > "$INITRD"
    rm -rf "$WORKDIR"
    cd "$ROOT"
    (cd "{{assets_dir}}" && b3sum vmlinuz initrd.img rootfs.img > B3SUMS)
    echo "initrd repacked (with agent + net-proxy + mcp-server + fs-watch)"
