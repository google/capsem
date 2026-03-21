#!/usr/bin/env bash
# Preflight checks for release builds.
# Validates environment, credentials, and tools BEFORE slow CI jobs run.
# Add new checks as functions -- they run in order, fail-fast on first error.
set -euo pipefail

PASS=0
FAIL=0
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")"

pass() { echo "  [PASS] $1"; PASS=$((PASS + 1)); }
fail() { echo "  [FAIL] $1"; FAIL=$((FAIL + 1)); }

# --------------------------------------------------------------------------
# Check: Apple certificate can be imported into a macOS keychain
# macOS `security import` only supports legacy PKCS12 (3DES/SHA1).
# OpenSSL 3.x creates PBES2/AES-256-CBC by default, which Keychain rejects
# with a misleading "wrong password" error. Re-export with:
#   openssl pkcs12 -in cert.p12 -passin pass:PWD -nodes -out combined.pem
#   openssl pkcs12 -export -in combined.pem -out cert.p12 -passout pass:PWD \
#     -certpbe PBE-SHA1-3DES -keypbe PBE-SHA1-3DES -macalg sha1
# --------------------------------------------------------------------------
check_apple_certificate() {
    echo ""
    echo "== Apple Certificate =="

    local cert_dir="$ROOT_DIR/private/apple-certificate"
    local p12="$cert_dir/capsem.p12"
    local pass_file="$cert_dir/p12-password.txt"

    if [[ ! -f "$p12" ]]; then
        fail "capsem.p12 not found at $p12"
        return
    fi
    pass "capsem.p12 exists"

    local password
    password="$(tr -d '\n' < "$pass_file")"

    # Check encryption format
    local fmt
    fmt=$(openssl pkcs12 -in "$p12" -info -nokeys -nocerts -passin "pass:$password" 2>&1 \
        | grep -o 'PBES2\|pbeWithSHA1And3-KeyTripleDES-CBC' | head -1)

    if [[ "$fmt" == "PBES2" ]]; then
        fail "p12 uses modern PBES2/AES encryption (macOS incompatible) -- run: scripts/fix_p12_legacy.sh"
        return
    fi
    pass "p12 uses legacy 3DES encryption (macOS-compatible)"

    # Try actual keychain import
    local keychain="preflight-$$.keychain"
    security create-keychain -p "" "$keychain" 2>/dev/null

    if ! security import "$p12" -k "$keychain" -P "$password" -T /usr/bin/codesign >/dev/null 2>&1; then
        security delete-keychain "$keychain" 2>/dev/null || true
        fail "keychain import failed"
        return
    fi
    pass "keychain import succeeded"

    security set-key-partition-list -S apple-tool:,apple: -k "" "$keychain" >/dev/null 2>&1

    local identity
    identity=$(security find-identity -v -p codesigning "$keychain" 2>/dev/null | grep "Developer ID" || true)
    security delete-keychain "$keychain" 2>/dev/null || true

    if [[ -z "$identity" ]]; then
        fail "no Developer ID signing identity found"
        return
    fi
    pass "signing identity: $(echo "$identity" | sed 's/.*"\(.*\)"/\1/')"
}

# --------------------------------------------------------------------------
# Check: base64-encoded certificate matches the p12 on disk
# --------------------------------------------------------------------------
check_b64_matches_p12() {
    echo ""
    echo "== Base64 Sync =="

    local cert_dir="$ROOT_DIR/private/apple-certificate"
    local p12="$cert_dir/capsem.p12"
    local b64="$cert_dir/capsem-b64.txt"

    if [[ ! -f "$b64" ]]; then
        fail "capsem-b64.txt not found"
        return
    fi

    local disk_b64
    disk_b64="$(base64 -i "$p12")"
    local file_b64
    file_b64="$(tr -d '\n\r ' < "$b64")"

    if [[ "$disk_b64" != "$file_b64" ]]; then
        fail "capsem-b64.txt does not match capsem.p12 -- regenerate with: base64 -i capsem.p12 -o capsem-b64.txt"
        return
    fi
    pass "capsem-b64.txt matches capsem.p12"
}

# --------------------------------------------------------------------------
# Check: required tools are available
# --------------------------------------------------------------------------
check_tools() {
    echo ""
    echo "== Required Tools =="

    local tools=(openssl codesign security cargo pnpm node gh)
    for tool in "${tools[@]}"; do
        if command -v "$tool" >/dev/null 2>&1; then
            pass "$tool"
        else
            fail "$tool not found"
        fi
    done
}

# --------------------------------------------------------------------------
# Check: Rust targets are installed
# --------------------------------------------------------------------------
check_rust_targets() {
    echo ""
    echo "== Rust Targets =="

    if rustup target list --installed 2>/dev/null | grep -q "aarch64-unknown-linux-musl"; then
        pass "aarch64-unknown-linux-musl installed"
    else
        fail "aarch64-unknown-linux-musl not installed -- run: rustup target add aarch64-unknown-linux-musl"
    fi
}


# --------------------------------------------------------------------------
# Check: Apple notarization credentials are present and work
# --------------------------------------------------------------------------
check_notarization() {
    echo ""
    echo "== Notarization =="

    local cert_dir="$ROOT_DIR/private/apple-certificate"
    local p8="$cert_dir/capsem.p8"
    local info="$cert_dir/api-key-info.txt"

    if [[ ! -f "$p8" ]]; then
        fail ".p8 key not found at $p8"
        return
    fi
    pass ".p8 key file exists"

    if [[ ! -f "$info" ]]; then
        fail "api-key-info.txt not found at $info"
        return
    fi

    local api_key api_issuer
    api_key=$(grep '^APPLE_API_KEY=' "$info" | head -1 | cut -d= -f2)
    api_issuer=$(grep '^APPLE_API_ISSUER=' "$info" | head -1 | cut -d= -f2)

    if [[ -z "$api_key" ]]; then
        fail "API Key ID not found in api-key-info.txt"
        return
    fi
    pass "API Key ID: $api_key"

    if [[ -z "$api_issuer" ]]; then
        fail "API Issuer ID not found in api-key-info.txt"
        return
    fi
    pass "API Issuer ID: $api_issuer"

    if ! command -v xcrun >/dev/null 2>&1; then
        fail "xcrun not found"
        return
    fi

    if ! xcrun notarytool --help >/dev/null 2>&1; then
        fail "xcrun notarytool not available"
        return
    fi
    pass "xcrun notarytool available"

    # Live check: verify credentials work against Apple's API (fast, no upload)
    if xcrun notarytool history \
        --key "$p8" \
        --key-id "$api_key" \
        --issuer "$api_issuer" \
        >/dev/null 2>&1; then
        pass "notarytool history succeeded (credentials valid)"
    else
        fail "notarytool history failed -- check .p8 key, Key ID, and Issuer ID"
    fi
}

# --------------------------------------------------------------------------
# Check: capsem-init does not allow state to persist between VM sessions.
# Invariants:
#   (1) scratch disk is always formatted unconditionally at boot (no ext4 detection skip)
#   (2) overlay upperdir is always on tmpfs, never on the scratch disk
# See docs/ephemeral_model.md for the incident that motivated these checks.
# --------------------------------------------------------------------------
check_ephemeral_model() {
    echo ""
    echo "== Ephemeral VM Model =="

    local init="$ROOT_DIR/images/capsem-init"

    if [[ ! -f "$init" ]]; then
        fail "capsem-init not found at $init"
        return
    fi

    # FAIL: conditional mke2fs skip (skip format if disk is already ext4)
    if grep -qE 'grep[[:space:]].*ext4|file[[:space:]].*ext4' "$init"; then
        fail "capsem-init conditionally skips mke2fs -- scratch disk would persist across reboots"
    else
        pass "capsem-init: no conditional mke2fs skip"
    fi

    # FAIL: scratch disk used as overlay upper layer
    if grep -qE 'UPPER=.*scratch|upperdir[=[:space:]].*scratch' "$init"; then
        fail "capsem-init uses scratch disk as overlayfs upper -- all rootfs writes would persist"
    else
        pass "capsem-init: scratch disk not used as overlay upper"
    fi

    # PASS: mke2fs must be present (scratch disk formatted at boot)
    if grep -q 'mke2fs' "$init"; then
        pass "capsem-init: mke2fs present (scratch disk formatted at every boot)"
    else
        fail "capsem-init: mke2fs missing -- scratch disk never formatted"
    fi

    # PASS: tmpfs used for overlay upper directory
    if grep -qE 'mount -t tmpfs tmpfs /mnt/b' "$init"; then
        pass "capsem-init: tmpfs used for overlay upper layer"
    else
        fail "capsem-init: tmpfs overlay upper not found -- writes may persist"
    fi

    # PASS: tmpfs mount failure must abort boot (no silent degraded mode)
    if grep -qE 'exit 1' "$init" && grep -A3 'mount -t tmpfs tmpfs /mnt/b' "$init" | grep -q 'exit 1'; then
        pass "capsem-init: tmpfs mount failure aborts boot (no silent degraded fallback)"
    else
        fail "capsem-init: tmpfs mount failure does not abort boot -- VM may start with wrong upper layer"
    fi
}

# --------------------------------------------------------------------------
# Check: every [[bin]] in capsem-agent is referenced in Dockerfile + justfile
# Source of truth: crates/capsem-agent/Cargo.toml [[bin]] entries.
# --------------------------------------------------------------------------
check_guest_binaries() {
    echo ""
    echo "== Guest Binaries =="

    local cargo_toml="$ROOT_DIR/crates/capsem-agent/Cargo.toml"
    local dockerfile="$ROOT_DIR/images/Dockerfile.rootfs"
    local justfile="$ROOT_DIR/justfile"

    if [[ ! -f "$cargo_toml" ]]; then
        fail "capsem-agent Cargo.toml not found at $cargo_toml"
        return
    fi

    # Extract [[bin]] name values from Cargo.toml
    local binaries
    binaries=$(grep -A1 '^\[\[bin\]\]' "$cargo_toml" | grep '^name' | sed 's/.*= *"\(.*\)"/\1/')

    if [[ -z "$binaries" ]]; then
        fail "no [[bin]] entries found in $cargo_toml"
        return
    fi

    for bin in $binaries; do
        # Check Dockerfile.rootfs
        if grep -q "COPY $bin " "$dockerfile"; then
            pass "Dockerfile.rootfs: $bin"
        else
            fail "Dockerfile.rootfs missing COPY for $bin"
        fi

        # Check justfile _pack-initrd section
        if grep -q "$bin" "$justfile"; then
            pass "justfile _pack-initrd: $bin"
        else
            fail "justfile missing $bin in _pack-initrd"
        fi
    done
}

# --------------------------------------------------------------------------
# Run all checks
# --------------------------------------------------------------------------
main() {
    echo "Capsem Release Preflight Checks"
    echo "================================"

    check_tools
    check_rust_targets
    check_apple_certificate
    check_b64_matches_p12
    check_notarization
    check_ephemeral_model
    check_guest_binaries

    echo ""
    echo "================================"
    echo "Results: $PASS passed, $FAIL failed"

    if [[ $FAIL -gt 0 ]]; then
        echo ""
        echo "Fix the failures above before releasing."
        exit 1
    fi

    echo "All preflight checks passed."
}

main "$@"
