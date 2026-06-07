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

die() {
    echo "ERROR: $*" >&2
    log "ERROR: $*"
    exit 1
}

# Platform check
if [[ "$(uname -s)" != "Darwin" ]]; then
    die "codesign requires macOS. VM features need macOS + Apple Silicon."
fi

# The first argument is the binary we need to sign and run.
if [ -f "$1" ]; then
    binary="$1"

    # Apply entitlements. Ad-hoc signing (-) is sufficient for local dev.
    if [ -f "$ENTITLEMENTS" ]; then
        log "signing $binary with entitlements"
        if ! codesign --sign - --entitlements "$ENTITLEMENTS" --force "$binary" >> "$BUILD_LOG" 2>&1; then
            die "codesign failed for $binary. Run 'just doctor' to diagnose signing issues."
        fi
        # Force the OS to re-evaluate the binary signature/entitlements
        touch "$binary"
    else
        die "entitlements.plist not found at $ENTITLEMENTS. Run 'just doctor' to diagnose."
    fi

    shift
    # Set the assets directory and execute the binary with remaining args.
    # CAPSEM_ASSETS_DIR allows the VM to find vmlinuz/initrd/rootfs.
    log "launching $binary $*"
    CAPSEM_ASSETS_DIR="$ROOT_DIR/assets" exec "$binary" "$@"
fi

# Fallback: just execute it.
exec "$@"
