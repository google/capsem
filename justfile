# Capsem Justfile

binary := "target/debug/capsem"
assets_dir := "assets"
entitlements := "entitlements.plist"

# Run the app in development mode with hot-reloading
dev:
    @echo "Stopping running instances..."
    -@pkill -x capsem 2>/dev/null || true
    -@pkill -x Capsem 2>/dev/null || true
    cargo tauri dev --config crates/capsem-app/tauri.conf.json

# Build VM assets (kernel, initrd, rootfs)
assets:
    cd images && python3 build.py

# Build frontend
frontend:
    cd frontend && pnpm build

# Compile the Rust binary
compile: frontend
    cargo build -p capsem

# Codesign the debug binary
sign: compile
    codesign --sign - --entitlements {{entitlements}} --force {{binary}}

# Run the signed debug binary
run: sign
    CAPSEM_ASSETS_DIR={{assets_dir}} {{binary}}

# Full rebuild: assets + app + sign, then smoke-test the VM boots
rebuild: assets sign
    CAPSEM_ASSETS_DIR={{assets_dir}} ./{{binary}} "echo capsem-ok"

# Build the release version of the app and sign it for macOS
build: frontend
    cd crates/capsem-app && cargo tauri build
    codesign --sign - --entitlements {{entitlements}} --force --deep \
        "target/release/bundle/macos/Capsem.app"

# Build and install the app to /Applications
install: build
    @echo "Stopping running Capsem..."
    -@pkill -x Capsem 2>/dev/null || true
    -@pkill -x capsem 2>/dev/null || true
    @echo "Installing to /Applications..."
    rm -rf "/Applications/Capsem.app"
    cp -R "target/release/bundle/macos/Capsem.app" "/Applications/"
    @echo "Launching Capsem..."
    open "/Applications/Capsem.app"

# Clean build artifacts
clean:
    cargo clean
    cd frontend && rm -rf build .svelte-kit node_modules
    rm -rf target/release/bundle/macos/Capsem.app

# Quick repack initrd + boot (fast debug cycle, ~5s)
# Repacks initrd with current capsem-init, rebuilds, codesigns, boots.
quick *CMD:
    #!/bin/bash
    set -euo pipefail
    ROOT="{{justfile_directory()}}"
    INITRD="$ROOT/{{assets_dir}}/initrd.img"
    if [ ! -f "$INITRD" ]; then
        echo "ERROR: $INITRD not found. Run 'just assets' first."
        exit 1
    fi
    echo "=== Repack initrd ==="
    WORKDIR=$(mktemp -d)
    cd "$WORKDIR"
    gzip -dc "$INITRD" | cpio -id 2>/dev/null
    cp "$ROOT/images/capsem-init" init
    chmod 755 init
    find . | cpio -o -H newc 2>/dev/null | gzip > "$INITRD"
    rm -rf "$WORKDIR"
    cd "$ROOT"
    (cd "{{assets_dir}}" && shasum -a 256 vmlinuz initrd.img rootfs.img > SHA256SUMS)
    echo "initrd repacked"
    echo ""
    echo "=== Build + sign ==="
    cargo build -p capsem 2>&1 | tail -3
    codesign --sign - --entitlements {{entitlements}} --force {{binary}}
    echo ""
    echo "=== Boot VM ==="
    CAPSEM_ASSETS_DIR={{assets_dir}} {{binary}} {{if CMD == "" { "echo capsem-ok" } else { CMD } }}

# Check code formatting and types
check:
    cargo check
    cd frontend && pnpm run build
