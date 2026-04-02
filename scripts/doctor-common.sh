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
# Ordered fix registry -- dependency order, deduped by design
# Checks mark entries as needed; --fix runs only the marked ones, in order.
# ---------------------------------------------------------------------------
FIX_IDS=()
FIX_CMDS=()
FIX_DESCS=()
FIX_NEEDED=()

_reg() { FIX_IDS+=("$1"); FIX_CMDS+=("$2"); FIX_DESCS+=("$3"); FIX_NEEDED+=(0); }

# Order matters: tools before builds, builds before assets
_reg rustup-targets   "rustup target add aarch64-unknown-linux-musl x86_64-unknown-linux-musl" \
                      "Install Rust cross-compile targets"
_reg llvm-tools       "rustup component add llvm-tools" \
                      "Install llvm-tools (provides rust-lld)"
_reg cargo-llvm-cov   "cargo install cargo-llvm-cov" \
                      "Install cargo-llvm-cov"
_reg cargo-audit      "cargo install cargo-audit" \
                      "Install cargo-audit"
_reg b3sum            "cargo install b3sum --locked" \
                      "Install b3sum"
_reg cargo-tauri      "cargo install cargo-tauri --locked" \
                      "Install cargo-tauri"
_reg entitlements     "git checkout entitlements.plist" \
                      "Restore entitlements.plist"
_reg cargo-config     "git checkout .cargo/config.toml" \
                      "Restore .cargo/config.toml"
_reg run-signed       "git checkout scripts/run_signed.sh && chmod +x scripts/run_signed.sh" \
                      "Restore scripts/run_signed.sh"
_reg run-signed-chmod "chmod +x scripts/run_signed.sh" \
                      "Make scripts/run_signed.sh executable"
_reg pnpm-install     "cd frontend && pnpm install --frozen-lockfile" \
                      "Install frontend deps"
_reg build-assets     "just build-assets" \
                      "Build VM assets (kernel + rootfs)"
_reg pack-initrd      "just _pack-initrd" \
                      "Cross-compile guest binaries + repack initrd"

need_fix() {
    local id="$1"
    for i in "${!FIX_IDS[@]}"; do
        if [[ "${FIX_IDS[$i]}" == "$id" ]]; then
            FIX_NEEDED[$i]=1
            return
        fi
    done
    echo "BUG: unknown fix id '$id'" >&2
}

# ---------------------------------------------------------------------------
# Counters
# ---------------------------------------------------------------------------
PASS=0; FAIL=0; SKIP=0; WARN=0

# Category tracking for recap
declare -a CAT_NAMES=()
declare -a CAT_PASS=()
declare -a CAT_FAIL=()
_cur_cat=""
_cur_pass=0
_cur_fail=0

section() {
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
    local fix_id="$1" label="$2"
    need_fix "$fix_id"
    fail "$label"
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
        fail "$tool not found -- install: $_hint"
    fi
done

section "Rust Toolchain"
for target in aarch64-unknown-linux-musl x86_64-unknown-linux-musl; do
    if rustup target list --installed 2>/dev/null | grep -q "$target"; then
        pass "target: $target"
    else
        fixable rustup-targets "target: $target missing"
    fi
done
if rustup component list --installed 2>/dev/null | grep -q llvm-tools; then
    pass "component: llvm-tools"
else
    fixable llvm-tools "component: llvm-tools missing"
fi

section "Cargo Tools"
_check_cargo_tool() {
    local tool="$1" fix_id="$2"
    if command -v "$tool" &>/dev/null; then
        pass "$tool"
    else
        fixable "$fix_id" "$tool not found"
    fi
}
_check_cargo_tool cargo-llvm-cov cargo-llvm-cov
_check_cargo_tool cargo-audit    cargo-audit
_check_cargo_tool b3sum          b3sum
_check_cargo_tool cargo-tauri    cargo-tauri

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
    fail "docker buildx not working -- install: $_hint"
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
            fixable build-assets "assets version mismatch: Cargo=$_cargo_ver, manifest=$_manifest_ver"
        fi

        if command -v b3sum &>/dev/null && [[ -f "$ASSETS_DIR/B3SUMS" ]]; then
            if (cd "$ASSETS_DIR" && b3sum --check B3SUMS >/dev/null 2>&1); then
                pass "asset integrity (B3SUMS match)"
            else
                fixable build-assets "asset integrity check failed"
            fi
        fi
    else
        fixable build-assets "manifest.json missing"
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
            fixable pack-initrd "$b found but not Linux ELF"
        fi
    else
        fixable pack-initrd "$b missing"
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
# Auto-fix: collect needed fixes in registry order, run them
# ---------------------------------------------------------------------------
_needed_count=0
for i in "${!FIX_IDS[@]}"; do
    if [[ "${FIX_NEEDED[$i]}" -eq 1 ]]; then
        _needed_count=$((_needed_count + 1))
    fi
done

if [[ "$_needed_count" -gt 0 ]]; then
    echo ""
    echo -e "${BOLD}${_needed_count} fix(es) available (in dependency order):${NC}"
    _n=1
    for i in "${!FIX_IDS[@]}"; do
        if [[ "${FIX_NEEDED[$i]}" -eq 1 ]]; then
            echo -e "  ${DIM}${_n}.${NC} ${FIX_DESCS[$i]} ${DIM}(${FIX_CMDS[$i]})${NC}"
            _n=$((_n + 1))
        fi
    done

    if [[ "${1:-}" == "--fix" ]]; then
        echo ""
        for i in "${!FIX_IDS[@]}"; do
            if [[ "${FIX_NEEDED[$i]}" -eq 1 ]]; then
                echo -e "${BOLD}Fixing:${NC} ${FIX_DESCS[$i]}"
                echo -e "  ${DIM}\$ ${FIX_CMDS[$i]}${NC}"
                if eval "${FIX_CMDS[$i]}"; then
                    echo -e "  ${GREEN}done${NC}"
                else
                    echo -e "  ${RED}failed${NC}"
                fi
                echo ""
            fi
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
