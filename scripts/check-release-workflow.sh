#!/usr/bin/env bash
# Verify release workflow steps locally before pushing.
# Catches tool/format/args issues that would waste a CI cycle.
set -euo pipefail

PASS=0; FAIL=0
pass() { echo "  OK  $1"; PASS=$((PASS + 1)); }
fail() { echo "  FAIL  $1"; FAIL=$((FAIL + 1)); }

echo "=== Release workflow preflight ==="

# --- Tools ---
echo ""
echo "Tools:"
command -v cargo >/dev/null && pass "cargo" || fail "cargo not found"
command -v minisign >/dev/null && pass "minisign" || fail "minisign not found (brew install minisign)"
cargo tauri --version >/dev/null 2>&1 && pass "cargo-tauri" || fail "cargo-tauri not found (cargo install tauri-cli)"
cargo sbom --help >/dev/null 2>&1 && pass "cargo-sbom" || fail "cargo-sbom not found (cargo install cargo-sbom)"

# --- Tauri key format ---
echo ""
echo "Tauri signing key:"
KEY_FILE="private/tauri/capsem.key"
if [ -f "$KEY_FILE" ]; then
    # The CI secret stores the key base64-encoded; verify decoding works
    KEY_B64=$(cat "$KEY_FILE")
    DECODED=$(echo "$KEY_B64" | base64 -d 2>/dev/null || true)
    if echo "$DECODED" | grep -q "rsign encrypted secret key"; then
        pass "key decodes to valid minisign format"
    else
        fail "key does not decode to minisign format -- check $KEY_FILE"
    fi
else
    fail "$KEY_FILE not found"
fi

# --- Manifest signing dry run ---
echo ""
echo "Manifest signing:"
if [ -f "assets/manifest.json" ] && [ -f "$KEY_FILE" ] && command -v minisign >/dev/null; then
    TMPKEY=$(mktemp)
    echo "$KEY_B64" | base64 -d > "$TMPKEY"
    # Read password from private/tauri/password if it exists
    PWD_FILE="private/tauri/password"
    if [ -f "$PWD_FILE" ]; then
        cat "$PWD_FILE" | minisign -S -s "$TMPKEY" -m assets/manifest.json 2>/dev/null && {
            pass "minisign signs manifest.json"
            rm -f assets/manifest.json.minisig
        } || fail "minisign failed to sign manifest.json"
    else
        echo "  SKIP  no password file at $PWD_FILE (can't test signing without it)"
    fi
    rm -f "$TMPKEY"
else
    echo "  SKIP  missing assets/manifest.json, key file, or minisign"
fi

# --- Tauri config: rootfs not bundled ---
echo ""
echo "Tauri config:"
if grep -q "rootfs" crates/capsem-app/tauri.conf.json; then
    fail "rootfs.squashfs is in tauri.conf.json resources -- must not be bundled in DMG"
else
    pass "rootfs not in DMG bundle resources"
fi

# --- Version sync ---
echo ""
echo "Version sync:"
CARGO_VER=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')
TAURI_VER=$(grep '"version"' crates/capsem-app/tauri.conf.json | sed 's/.*"\([0-9][^"]*\)".*/\1/')
if [ "$CARGO_VER" = "$TAURI_VER" ]; then
    pass "Cargo.toml ($CARGO_VER) == tauri.conf.json ($TAURI_VER)"
else
    fail "version mismatch: Cargo.toml=$CARGO_VER, tauri.conf.json=$TAURI_VER"
fi

# --- Summary ---
echo ""
echo "=== $PASS passed, $FAIL failed ==="
[ "$FAIL" -eq 0 ] || exit 1
