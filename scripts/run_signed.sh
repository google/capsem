#!/bin/bash
# scripts/run_signed.sh
#
# Custom runner for Capsem development.
# Handles signing the binary with Virtualization entitlements on macOS.
# All runner diagnostics go to a unified build log (never stdout/stderr).

# Find the workspace root based on the script's location
DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )"
ROOT_DIR="$(dirname "$DIR")"
ENTITLEMENTS="$ROOT_DIR/entitlements.plist"
BUILD_LOG="$ROOT_DIR/target/build.log"

# Ensure target/ exists (cargo creates it, but just in case)
mkdir -p "$ROOT_DIR/target"

log() {
    echo "[runner] $(date +%H:%M:%S) $*" >> "$BUILD_LOG"
}

# The first argument is the binary we need to sign and run.
if [ -f "$1" ]; then
    binary="$1"

    # Apply entitlements. Ad-hoc signing (-) is sufficient for local dev.
    if [ -f "$ENTITLEMENTS" ]; then
        log "signing $binary with entitlements"
        codesign --sign - --entitlements "$ENTITLEMENTS" --force "$binary" >> "$BUILD_LOG" 2>&1
        # Force the OS to re-evaluate the binary signature/entitlements
        touch "$binary"
    else
        log "WARNING: entitlements.plist not found at $ENTITLEMENTS, signing without it"
        codesign --sign - --force "$binary" >> "$BUILD_LOG" 2>&1
        touch "$binary"
    fi

    shift
    # Set the assets directory and execute the binary with remaining args.
    # CAPSEM_ASSETS_DIR allows the VM to find vmlinuz/initrd/rootfs.
    log "launching $binary $*"
    CAPSEM_ASSETS_DIR="$ROOT_DIR/assets" exec "$binary" "$@"
fi

# Fallback: just execute it.
exec "$@"
