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

# --- Manifest signing dry run ---
echo ""
echo "Manifest signing:"
MANIFEST_PUBKEY="config/manifest-sign.pub"
DEFAULT_MANIFEST_KEY_FILE="private/manifest-sign/capsem.key"
FALLBACK_MANIFEST_KEY_FILE="private/minisign/manifest.key"
if [ -n "${MANIFEST_SIGN_KEY_FILE:-}" ]; then
    MANIFEST_KEY_FILE="$MANIFEST_SIGN_KEY_FILE"
elif [ -f "$DEFAULT_MANIFEST_KEY_FILE" ]; then
    MANIFEST_KEY_FILE="$DEFAULT_MANIFEST_KEY_FILE"
elif [ -f "$FALLBACK_MANIFEST_KEY_FILE" ]; then
    MANIFEST_KEY_FILE="$FALLBACK_MANIFEST_KEY_FILE"
else
    MANIFEST_KEY_FILE="$DEFAULT_MANIFEST_KEY_FILE"
fi
DEFAULT_MANIFEST_PASSWORD_FILE="private/manifest-sign/password"
FALLBACK_MANIFEST_PASSWORD_FILE="private/minisign/password"
if [ -n "${MANIFEST_SIGN_PASSWORD_FILE:-}" ]; then
    MANIFEST_PASSWORD_FILE="$MANIFEST_SIGN_PASSWORD_FILE"
elif [ -f "$DEFAULT_MANIFEST_PASSWORD_FILE" ]; then
    MANIFEST_PASSWORD_FILE="$DEFAULT_MANIFEST_PASSWORD_FILE"
elif [ -f "$FALLBACK_MANIFEST_PASSWORD_FILE" ]; then
    MANIFEST_PASSWORD_FILE="$FALLBACK_MANIFEST_PASSWORD_FILE"
else
    MANIFEST_PASSWORD_FILE=""
fi
if [ ! -f "$MANIFEST_PUBKEY" ]; then
    fail "$MANIFEST_PUBKEY not found"
elif [ ! -f "$MANIFEST_KEY_FILE" ]; then
    fail "$MANIFEST_KEY_FILE not found (set MANIFEST_SIGN_KEY_FILE to override)"
elif [ ! -f "assets/manifest.json" ]; then
    fail "assets/manifest.json not found"
elif ! command -v minisign >/dev/null; then
    fail "minisign not found"
else
    TMPDIR=$(mktemp -d)
    TMPMANIFEST="$TMPDIR/manifest.json"
    TMPSIG="$TMPDIR/manifest.json.minisig"
    cp assets/manifest.json "$TMPMANIFEST"
    SIGNED=0
    if [ -n "$MANIFEST_PASSWORD_FILE" ] && [ -f "$MANIFEST_PASSWORD_FILE" ]; then
        if minisign -S -s "$MANIFEST_KEY_FILE" -m "$TMPMANIFEST" -x "$TMPSIG" < "$MANIFEST_PASSWORD_FILE" >/dev/null 2>&1; then
            pass "manifest key signs manifest.json"
            SIGNED=1
        else
            fail "manifest key failed to sign manifest.json"
        fi
    elif [ -n "${MINISIGN_PASSWORD:-}" ]; then
        if printf '%s\n' "$MINISIGN_PASSWORD" | minisign -S -s "$MANIFEST_KEY_FILE" -m "$TMPMANIFEST" -x "$TMPSIG" >/dev/null 2>&1; then
            pass "manifest key signs manifest.json"
            SIGNED=1
        else
            fail "manifest key failed to sign manifest.json"
        fi
    elif minisign -S -s "$MANIFEST_KEY_FILE" -m "$TMPMANIFEST" -x "$TMPSIG" </dev/null >/dev/null 2>&1; then
        pass "manifest key signs manifest.json (passwordless key)"
        SIGNED=1
    else
        fail "manifest key failed to sign manifest.json (if encrypted, set MANIFEST_SIGN_PASSWORD_FILE or MINISIGN_PASSWORD)"
    fi

    if [ "$SIGNED" -eq 1 ] && minisign -Vm "$TMPMANIFEST" -x "$TMPSIG" -p "$MANIFEST_PUBKEY" >/dev/null 2>&1; then
        pass "manifest signature verifies with $MANIFEST_PUBKEY"
    elif [ "$SIGNED" -eq 1 ]; then
        fail "manifest signing key does not match $MANIFEST_PUBKEY"
    else
        fail "manifest signature verification skipped because signing failed"
    fi
    rm -rf "$TMPDIR"
fi

# --- Updater strategy ---
echo ""
echo "Updater strategy:"
UPDATER_MATCHES=$(grep -R -n -E 'createUpdaterArtifacts|latest\.json|tauri-plugin-updater|tauri_plugin_updater|updater:default' \
    crates/capsem-app/src \
    crates/capsem-app/tauri.conf.json \
    crates/capsem-app/capabilities \
    frontend/src/lib/api.ts \
    frontend/src/lib/components/settings \
    frontend/src/lib/components/shell/SettingsPage.svelte 2>/dev/null || true)
if [ -n "$UPDATER_MATCHES" ]; then
    echo "$UPDATER_MATCHES"
    fail "unsupported Tauri updater surface is still enabled"
else
    pass "unsupported Tauri updater surface disabled"
fi

# --- Release workflow policy ---
echo ""
echo "Release workflow policy:"
if grep -q 'continue-on-error: true' .github/workflows/release.yaml; then
    fail "release workflow contains continue-on-error"
else
    pass "release workflow has no continue-on-error"
fi
if grep -qiE 'best-effort|skipping binary e2e|no \.deb.*skipping' .github/workflows/release.yaml; then
    fail "release workflow contains optional package publishing/proof wording"
else
    pass "package publishing/proof is release-blocking"
fi
if grep -q 'scripts/validate-rootfs.sh assets/${{ matrix.arch }}/rootfs.squashfs' .github/workflows/release.yaml \
    && grep -q 'GUEST_BINARIES' scripts/validate-rootfs.sh \
    && grep -q 'ROOTFS_SCRIPTS' scripts/validate-rootfs.sh; then
    pass "rootfs validation uses canonical artifact lists"
else
    fail "rootfs validation is not wired to canonical artifact lists"
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
PYPROJECT_VER=$(grep '^version' pyproject.toml | head -1 | sed 's/.*"\(.*\)"/\1/')
if [ "$CARGO_VER" = "$TAURI_VER" ]; then
    pass "Cargo.toml ($CARGO_VER) == tauri.conf.json ($TAURI_VER)"
else
    fail "version mismatch: Cargo.toml=$CARGO_VER, tauri.conf.json=$TAURI_VER"
fi
if [ "$CARGO_VER" = "$PYPROJECT_VER" ]; then
    pass "Cargo.toml ($CARGO_VER) == pyproject.toml ($PYPROJECT_VER)"
else
    fail "version mismatch: Cargo.toml=$CARGO_VER, pyproject.toml=$PYPROJECT_VER"
fi

# --- Summary ---
echo ""
echo "=== $PASS passed, $FAIL failed ==="
[ "$FAIL" -eq 0 ] || exit 1
