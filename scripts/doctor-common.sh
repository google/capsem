#!/bin/bash
# Capsem Doctor -- development environment health check
# Usage: scripts/doctor-common.sh [--fix]
#   --fix  Auto-fix all fixable issues without prompting
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

ENTITLEMENTS="entitlements.plist"
ASSETS_DIR="assets"

# ---------------------------------------------------------------------------
# Colors (disabled when not a TTY)
# ---------------------------------------------------------------------------
if [[ -t 1 ]]; then
    GREEN='\033[0;32m'; RED='\033[0;31m'; YELLOW='\033[0;33m'
    BOLD='\033[1m'; DIM='\033[2m'; NC='\033[0m'
else
    GREEN=''; RED=''; YELLOW=''; BOLD=''; DIM=''; NC=''
fi

# ---------------------------------------------------------------------------
# Counters and fix collection
# ---------------------------------------------------------------------------
PASS=0; FAIL=0; SKIP=0; WARN=0
FIX_CMDS=()
FIX_LABELS=()

# Category tracking for recap
declare -a CAT_NAMES=()
declare -a CAT_PASS=()
declare -a CAT_FAIL=()
_cur_cat=""
_cur_pass=0
_cur_fail=0

section() {
    # Close previous category
    if [[ -n "$_cur_cat" ]]; then
        CAT_NAMES+=("$_cur_cat")
        CAT_PASS+=("$_cur_pass")
        CAT_FAIL+=("$_cur_fail")
    fi
    _cur_cat="$1"
    _cur_pass=0
    _cur_fail=0
    echo ""
    echo -e "${BOLD}== $1 ==${NC}"
}

pass() {
    echo -e "  ${GREEN}[PASS]${NC} $1"
    PASS=$((PASS + 1))
    _cur_pass=$((_cur_pass + 1))
}

fail() {
    echo -e "  ${RED}[FAIL]${NC} $1"
    FAIL=$((FAIL + 1))
    _cur_fail=$((_cur_fail + 1))
}

warn() {
    echo -e "  ${YELLOW}[WARN]${NC} $1"
    WARN=$((WARN + 1))
}

skip() {
    echo -e "  ${DIM}[SKIP]${NC} $1"
    SKIP=$((SKIP + 1))
}

fixable() {
    local cmd="$1" label="$2"
    fail "$label -- fix: $cmd"
    FIX_CMDS+=("$cmd")
    FIX_LABELS+=("$label")
}

# ---------------------------------------------------------------------------
# Load platform-specific checks
# ---------------------------------------------------------------------------
if [[ "$(uname -s)" == "Darwin" ]]; then
    source "$SCRIPT_DIR/doctor-macos.sh"
else
    source "$SCRIPT_DIR/doctor-linux.sh"
fi

# ---------------------------------------------------------------------------
# Cross-platform checks
# ---------------------------------------------------------------------------
echo -e "${BOLD}Capsem Doctor${NC}"
echo "============================================"

section "System Tools"
for tool in cargo rustup node python3 uv pnpm sqlite3 git b3sum; do
    if command -v "$tool" &>/dev/null; then
        pass "$tool"
    else
        _hint=$(tool_hint "$tool")
        if [[ -n "$_hint" ]]; then
            fixable "$_hint" "$tool not found"
        else
            fail "$tool not found"
        fi
    fi
done

section "Rust Toolchain"
for target in aarch64-unknown-linux-musl x86_64-unknown-linux-musl; do
    if rustup target list --installed 2>/dev/null | grep -q "$target"; then
        pass "target: $target"
    else
        fixable "rustup target add $target" "target: $target missing"
    fi
done
if rustup component list --installed 2>/dev/null | grep -q llvm-tools; then
    pass "component: llvm-tools"
else
    fixable "rustup component add llvm-tools" "component: llvm-tools missing"
fi

section "Cargo Tools"
_check_cargo_tool() {
    local tool="$1" install_cmd="$2"
    if command -v "$tool" &>/dev/null; then
        pass "$tool"
    else
        fixable "$install_cmd" "$tool not found"
    fi
}
_check_cargo_tool cargo-llvm-cov "cargo install cargo-llvm-cov"
_check_cargo_tool cargo-audit    "cargo install cargo-audit"
_check_cargo_tool b3sum          "cargo install b3sum --locked"
_check_cargo_tool cargo-tauri    "cargo install cargo-tauri --locked"

section "Docker"
if command -v docker &>/dev/null; then
    pass "docker ($(docker --version 2>/dev/null | head -1))"
else
    _hint=$(tool_hint docker)
    fail "docker not found -- install: $_hint"
fi

if docker buildx version &>/dev/null; then
    pass "docker buildx ($(docker buildx version 2>/dev/null | head -1))"
else
    _hint=$(tool_hint docker-buildx)
    if [[ -n "$_hint" ]]; then
        fixable "$_hint" "docker buildx not working"
    else
        fail "docker buildx not working"
    fi
fi

# Platform-specific checks (colima, codesigning, KVM, etc.)
check_platform

section "VM Assets"
if [[ -z "${CAPSEM_SKIP_ASSET_CHECK:-}" ]]; then
    if [[ -f "$ASSETS_DIR/manifest.json" ]]; then
        _cargo_ver=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')
        _manifest_ver=$(grep '"latest":' "$ASSETS_DIR/manifest.json" | sed 's/.*: "\(.*\)".*/\1/')
        if [[ "$_cargo_ver" == "$_manifest_ver" ]]; then
            pass "assets version ($_manifest_ver) matches Cargo.toml"
        else
            fail "assets version mismatch: Cargo=$_cargo_ver, manifest=$_manifest_ver -- run: just build-assets"
        fi

        if command -v b3sum &>/dev/null && [[ -f "$ASSETS_DIR/B3SUMS" ]]; then
            if (cd "$ASSETS_DIR" && b3sum --check B3SUMS >/dev/null 2>&1); then
                pass "asset integrity (B3SUMS match)"
            else
                fail "asset integrity check failed -- run: just build-assets"
            fi
        fi
    else
        fail "manifest.json missing -- run: just build-assets"
    fi
else
    skip "VM Assets (CAPSEM_SKIP_ASSET_CHECK set)"
fi

section "Guest Binaries"
arch=$(uname -m | sed 's/aarch64/arm64/')
release_dir="target/linux-agent/$arch"
for b in capsem-pty-agent capsem-net-proxy capsem-mcp-server; do
    if [[ -f "$release_dir/$b" ]]; then
        if file "$release_dir/$b" 2>/dev/null | grep -E -q "ELF 64-bit"; then
            pass "$b (Linux ELF)"
        else
            fail "$b found but not Linux ELF -- run: just _pack-initrd"
        fi
    else
        fail "$b missing -- run: just _pack-initrd"
    fi
done

section "Release Tools"
for tool in gh openssl minisign cargo-sbom; do
    if command -v "$tool" &>/dev/null; then
        pass "$tool"
    else
        skip "$tool (only needed for releases)"
    fi
done

# ---------------------------------------------------------------------------
# Close final category and show recap
# ---------------------------------------------------------------------------
if [[ -n "$_cur_cat" ]]; then
    CAT_NAMES+=("$_cur_cat")
    CAT_PASS+=("$_cur_pass")
    CAT_FAIL+=("$_cur_fail")
fi

echo ""
echo "============================================"
echo -e "${BOLD}  Capsem Doctor Results${NC}"
echo "============================================"
for i in "${!CAT_NAMES[@]}"; do
    _p="${CAT_PASS[$i]}"
    _f="${CAT_FAIL[$i]}"
    _total=$(( _p + _f ))
    if [[ "$_f" -eq 0 ]]; then
        _status="${GREEN}${_p}/${_total}${NC}"
    else
        _status="${RED}${_p}/${_total}${NC}"
    fi
    printf "  %-22s %b passed\n" "${CAT_NAMES[$i]}" "$_status"
done
echo "--------------------------------------------"
if [[ "$FAIL" -eq 0 ]]; then
    echo -e "  ${GREEN}${BOLD}$PASS passed${NC}, $SKIP skipped, $WARN warnings"
else
    echo -e "  ${GREEN}$PASS passed${NC}, ${RED}${BOLD}$FAIL failed${NC}, $SKIP skipped, $WARN warnings"
fi
echo "============================================"

# ---------------------------------------------------------------------------
# Auto-fix
# ---------------------------------------------------------------------------
if [[ ${#FIX_CMDS[@]} -gt 0 ]]; then
    echo ""
    echo -e "${BOLD}${#FIX_CMDS[@]} issue(s) can be auto-fixed:${NC}"
    for i in "${!FIX_LABELS[@]}"; do
        echo -e "  ${DIM}$((i+1)).${NC} ${FIX_LABELS[$i]}"
    done
    echo ""

    if [[ "${1:-}" == "--fix" ]]; then
        echo ""
        for i in "${!FIX_CMDS[@]}"; do
            echo -e "${BOLD}Fixing:${NC} ${FIX_LABELS[$i]}"
            echo -e "  ${DIM}\$ ${FIX_CMDS[$i]}${NC}"
            if eval "${FIX_CMDS[$i]}"; then
                echo -e "  ${GREEN}done${NC}"
            else
                echo -e "  ${RED}failed${NC}"
            fi
            echo ""
        done
        echo -e "${BOLD}Re-running doctor to verify...${NC}"
        echo ""
        exec "$0"
    else
        echo ""
        echo -e "Run ${BOLD}just doctor-fix${NC} to auto-fix these issues."
    fi
fi

# ---------------------------------------------------------------------------
# Exit
# ---------------------------------------------------------------------------
if [[ "$FAIL" -gt 0 ]]; then
    exit 1
fi
touch .dev-setup
