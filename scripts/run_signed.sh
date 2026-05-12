#!/bin/bash
# scripts/run_signed.sh
#
# Custom runner for Capsem development.
# Handles signing the binary with Virtualization entitlements on macOS.
# Normal runner diagnostics go to a unified build log; failures include a
# short tail on stderr so CI logs preserve the root cause.

# Find the workspace root based on the script's location
DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )"
ROOT_DIR="$(dirname "$DIR")"
ENTITLEMENTS="$ROOT_DIR/entitlements.plist"
BUILD_LOG="$ROOT_DIR/target/build.log"
SIGN_LOCK_DIR="$ROOT_DIR/target/run-signed.codesign.lock"

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

die_with_log_tail() {
    echo "ERROR: $*" >&2
    log "ERROR: $*"
    if [ -f "$BUILD_LOG" ]; then
        echo "---- tail of $BUILD_LOG ----" >&2
        tail -40 "$BUILD_LOG" >&2 || true
        echo "---- end $BUILD_LOG ----" >&2
    fi
    exit 1
}

release_codesign_lock() {
    rm -f "$SIGN_LOCK_DIR/pid" 2>/dev/null || true
    rmdir "$SIGN_LOCK_DIR" 2>/dev/null || true
    trap - EXIT
}

acquire_codesign_lock() {
    local attempts=0

    while ! mkdir "$SIGN_LOCK_DIR" 2>/dev/null; do
        if [ -f "$SIGN_LOCK_DIR/pid" ]; then
            local owner
            owner="$(cat "$SIGN_LOCK_DIR/pid" 2>/dev/null || true)"
            if [ -n "$owner" ] && ! kill -0 "$owner" 2>/dev/null; then
                log "removing stale codesign lock owned by pid $owner"
                rm -rf "$SIGN_LOCK_DIR"
                continue
            fi
        fi

        attempts=$((attempts + 1))
        if [ "$attempts" -ge 600 ]; then
            die_with_log_tail "timed out waiting for codesign lock at $SIGN_LOCK_DIR"
        fi
        sleep 0.1
    done

    echo "$$" > "$SIGN_LOCK_DIR/pid"
    trap release_codesign_lock EXIT
}

signed_with_entitlements() {
    local binary="$1"

    codesign --verify "$binary" >> "$BUILD_LOG" 2>&1 \
        && codesign -d --entitlements - "$binary" 2>> "$BUILD_LOG" \
            | grep -q "com.apple.security.virtualization"
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
        acquire_codesign_lock
        if signed_with_entitlements "$binary"; then
            log "already signed with entitlements: $binary"
        else
            log "signing $binary with entitlements"
            if ! codesign --sign - --entitlements "$ENTITLEMENTS" --force "$binary" >> "$BUILD_LOG" 2>&1; then
                die_with_log_tail "codesign failed for $binary. Run 'just doctor' to diagnose signing issues."
            fi
        fi
        release_codesign_lock
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
