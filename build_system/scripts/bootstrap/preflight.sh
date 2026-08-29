#!/usr/bin/env bash
# Preflight checks for release builds.
# Validates environment, credentials, and tools BEFORE slow CI jobs run.
# Add new checks as functions -- they run in order, fail-fast on first error.
set -euo pipefail

PASS=0
FAIL=0
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"

pass() { echo "  [PASS] $1"; PASS=$((PASS + 1)); }
fail() { echo "  [FAIL] $1"; FAIL=$((FAIL + 1)); }

# --------------------------------------------------------------------------
# Check: required tools are available
# --------------------------------------------------------------------------
check_tools() {
    echo ""
    echo "== Required Tools =="

    local tools=(openssl codesign security cargo pnpm node gh uv)
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

    if rustup target list --installed 2>/dev/null | grep -q "x86_64-unknown-linux-musl"; then
        pass "x86_64-unknown-linux-musl installed"
    else
        fail "x86_64-unknown-linux-musl not installed -- run: rustup target add x86_64-unknown-linux-musl"
    fi
}


source "$SCRIPT_DIR/preflight-apple.sh"
source "$SCRIPT_DIR/preflight-source.sh"

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
