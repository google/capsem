#!/bin/bash
# scripts/run_signed.sh
#
# Custom runner for Capsem development. 
# Handles signing the binary with Virtualization entitlements on macOS.

# Find the workspace root based on the script's location
DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )"
ROOT_DIR="$(dirname "$DIR")"
ENTITLEMENTS="$ROOT_DIR/entitlements.plist"

# The first argument is the binary we need to sign and run.
if [ -f "$1" ]; then
    binary="$1"
    
    # Apply entitlements. Ad-hoc signing (-) is sufficient for local dev.
    if [ -f "$ENTITLEMENTS" ]; then
        echo "[runner] signing $binary with entitlements"
        codesign --sign - --entitlements "$ENTITLEMENTS" --force "$binary"
        # Force the OS to re-evaluate the binary signature/entitlements
        touch "$binary"
    else
        echo "Warning: entitlements.plist not found at $ENTITLEMENTS, signing without it."
        codesign --sign - --force "$binary"
        touch "$binary"
    fi
    
    shift
    # Set the assets directory and execute the binary with remaining args.
    # CAPSEM_ASSETS_DIR allows the VM to find vmlinuz/initrd/rootfs.
    echo "[runner] launching $binary"
    CAPSEM_ASSETS_DIR="$ROOT_DIR/assets" exec "$binary" "$@"
fi

# Fallback: just execute it.
exec "$@"
