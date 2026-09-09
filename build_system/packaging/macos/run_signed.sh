#!/bin/bash
# build_system/packaging/macos/run_signed.sh
#
# Custom runner for Capsem development.
# Handles signing the binary with Virtualization entitlements on macOS.
# All runner diagnostics go to a unified build log (never stdout/stderr).
set -o pipefail

# Rust's macOS pipe() + separate CLOEXEC update can race nextest's parallel
# spawns. Close foreign inherited descriptors before launching any helpers.
# This affects only nextest entry, not children created later by a test or
# descriptors intentionally passed to ordinary cargo run. Bash owns fd 255
# for this script; stdin/stdout/stderr belong to nextest's capture contract.
if [[ "${NEXTEST:-}" == "1" ]]; then
    for descriptor_path in /dev/fd/*; do
        descriptor=${descriptor_path##*/}
        case "$descriptor" in 0|1|2|255|*[!0-9]*) continue ;; esac
        eval "exec ${descriptor}>&-"
    done
fi

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
    trap 'if [[ -n "$staging" ]]; then rm -f "$staging"; fi; rm -rf "$SIGN_LOCK_DIR"' EXIT
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
    [[ -f "$binary" && -f "$receipt" ]] || return 1
    fingerprint=$(stat -f '%d:%i:%p:%z:%.9Fm:%.9Fc' "$binary") || return 1
    [[ -f "$receipt" && "$(< "$receipt")" == "$fingerprint" ]]
}

source_identity() {
    # Include inode, mode and nanosecond ctime so replacing a binary or
    # restoring its mtime cannot hide a write. Policy changes invalidate reuse.
    stat -f '%d:%i:%p:%z:%.9Fm:%.9Fc' "$original" "$ENTITLEMENTS" "${BASH_SOURCE[0]}"
}

prepare_signed_copy() {
    local source key published captured copied
    for attempt in 1 2 3; do
        if ! source=$(source_identity 2>> "$BUILD_LOG"); then
            [[ -f "$original" ]] && die "cannot stat signing inputs for $original"
            log "Cargo removed $original before capture $attempt; retrying"
            sleep 0.05
            continue
        fi
        key=$(printf '%s\n%s' "$original" "$source" | shasum -a 256) || die "cannot identify $original"
        key=${key%% *}
        # Keep the executable beside Cargo's original: current_exe().parent()
        # must still discover sibling service/process binaries. APFS clones
        # share storage, but codesign never mutates Cargo's alias or inode.
        published="$binary_dir/.run-signed-${original##*/}-$key"
        binary="$published"
        receipt="$SIGN_CACHE_DIR/$key"
        signature_receipt_current && return
        acquire_sign_lock
        if ! signature_receipt_current; then
            staging="$published.tmp.$$"
            copied=0
            cp -c "$original" "$staging" 2>> "$BUILD_LOG" && copied=1
            if ! captured=$(source_identity 2>> "$BUILD_LOG"); then
                [[ -f "$original" ]] && die "cannot recheck signing inputs for $original"
                captured=""
            fi
            if [[ "$captured" != "$source" ]]; then
                # A concurrent Cargo build changed the source while copying.
                # Retry only that observed race, before signing or publishing.
                log "Cargo replaced $original during capture $attempt; retrying"
                rm -f "$staging"
                staging=""
                release_sign_lock
                sleep 0.05
                continue
            fi
            [[ "$copied" == 1 ]] || die "cannot capture $original"
            binary="$staging"
            expected=$(plutil -convert xml1 -o - "$ENTITLEMENTS") || die "invalid entitlements at $ENTITLEMENTS"
            if ! signature_current; then
                log "signing captured $original with entitlements"
                codesign --sign - --entitlements "$ENTITLEMENTS" --force "$binary" >> "$BUILD_LOG" 2>&1 ||
                    die "codesign failed for $original. Run 'just doctor' to diagnose signing issues."
                signature_current || die "codesign verification failed for $original"
            fi
            mv "$staging" "$published" || die "cannot publish signed $original"
            staging=""
            binary="$published"
            stat -f '%d:%i:%p:%z:%.9Fm:%.9Fc' "$binary" > "$receipt.tmp" || die "cannot record signature for $original"
            mv "$receipt.tmp" "$receipt" || die "cannot publish signature for $original"
        fi
        release_sign_lock
        return
    done
    die "Cargo kept replacing $original while capturing its executable"
}

# Platform check
if [[ "$(uname -s)" != "Darwin" ]]; then
    die "codesign requires macOS. VM features need macOS + Apple Silicon."
fi

# The first argument is the binary we need to sign and run.
if [ -f "$1" ]; then
    original="$1"
    staging=""

    # Apply entitlements. Ad-hoc signing (-) is sufficient for local dev.
    if [ -f "$ENTITLEMENTS" ]; then
        # This receipt belongs to the compiled artifact, inside Cargo's owned
        # output directory. It has no independent retained cache authority.
        binary_dir="$(dirname "$original")"
        SIGN_CACHE_DIR="$binary_dir/.run-signed"
        mkdir -p "$SIGN_CACHE_DIR" || die "cannot create signature receipt directory"
        prepare_signed_copy
    else
        die "entitlements.plist not found at $ENTITLEMENTS. Run 'just doctor' to diagnose."
    fi

    shift
    # Set the assets directory and execute the binary with remaining args.
    # CAPSEM_ASSETS_DIR allows the VM to find vmlinuz/initrd/rootfs.
    log "launching $binary $*"
    CAPSEM_ASSETS_DIR="$ROOT_DIR/assets" exec -a "$original" "$binary" "$@"
fi

# Fallback: just execute it.
exec "$@"
