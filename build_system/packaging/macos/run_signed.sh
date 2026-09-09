#!/bin/bash
# build_system/packaging/macos/run_signed.sh
#
# Custom runner for Capsem development.
# Handles signing the binary with Virtualization entitlements on macOS.
# All runner diagnostics go to a unified build log (never stdout/stderr).
set -o pipefail

# Find the workspace root based on the script's owned package location.
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )"
ROOT_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
ENTITLEMENTS="$SCRIPT_DIR/entitlements.plist"
BUILD_LOG="$ROOT_DIR/cache/containers/logs/build.log"
SIGN_LOCK_DIR="$ROOT_DIR/cache/target/.run_signed_codesign.lock"

# The runner owns these two cache leaves and must work in a fresh checkout.
mkdir -p "$(dirname "$BUILD_LOG")" "$(dirname "$SIGN_LOCK_DIR")"

log() {
    echo "[runner] $(date +%H:%M:%S) $*" >> "$BUILD_LOG"
}

die() {
    echo "ERROR: $*" >&2
    log "ERROR: $*"
    exit 1
}

acquire_sign_lock() {
    local attempts=0
    while ! mkdir "$SIGN_LOCK_DIR" 2>/dev/null; do
        attempts=$((attempts + 1))
        if [ "$attempts" -ge 600 ]; then
            die "timed out waiting for codesign lock at $SIGN_LOCK_DIR"
        fi
        sleep 0.05
    done
    trap 'rm -rf "$SIGN_LOCK_DIR"' EXIT
}

release_sign_lock() {
    rm -rf "$SIGN_LOCK_DIR"
    trap - EXIT
}

signature_current() {
    local actual
    actual=$(codesign --display --entitlements - --xml "$binary" 2>> "$BUILD_LOG" |
        plutil -convert xml1 -o - - 2>> "$BUILD_LOG") || return 1
    [[ "$actual" == "$expected" ]] &&
        codesign --verify --strict "$binary" >> "$BUILD_LOG" 2>&1
}

signature_receipt_current() {
    # ctime catches writes even when a compiler restores mtime; inode catches
    # atomic replacements. Include the policy and runner so either invalidates
    # reuse. BSD stat's explicit precision preserves nanoseconds on APFS.
    fingerprint=$(stat -f '%d:%i:%p:%z:%.9Fm:%.9Fc' "$binary" "$ENTITLEMENTS" "${BASH_SOURCE[0]}") || return 1
    [[ -f "$receipt" && "$(< "$receipt")" == "$fingerprint" ]]
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
        key=$(stat -f '%d-%i' "$binary") || die "cannot stat $binary"
        # This receipt belongs to the compiled artifact, inside Cargo's owned
        # output directory. It has no independent retained cache authority.
        SIGN_CACHE_DIR="$(dirname "$binary")/.run-signed"
        mkdir -p "$SIGN_CACHE_DIR" || die "cannot create signature receipt directory"
        receipt="$SIGN_CACHE_DIR/$key"
        # Nextest invokes this runner once per test. Verify without changing the
        # executable; re-signing thousands of times serialized all test starts.
        if ! signature_receipt_current; then
            acquire_sign_lock
            # Another cold launch may have signed this binary while we waited.
            if ! signature_receipt_current; then
                expected=$(plutil -convert xml1 -o - "$ENTITLEMENTS") || die "invalid entitlements at $ENTITLEMENTS"
                if ! signature_current; then
                    log "signing $binary with entitlements"
                    if ! codesign --sign - --entitlements "$ENTITLEMENTS" --force "$binary" >> "$BUILD_LOG" 2>&1; then
                        die "codesign failed for $binary. Run 'just doctor' to diagnose signing issues."
                    fi
                    signature_current || die "codesign verification failed for $binary"
                fi
                stat -f '%d:%i:%p:%z:%.9Fm:%.9Fc' "$binary" "$ENTITLEMENTS" "${BASH_SOURCE[0]}" > "$receipt.tmp" || die "cannot record signature for $binary"
                mv "$receipt.tmp" "$receipt" || die "cannot publish signature for $binary"
            fi
            release_sign_lock
        fi
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
