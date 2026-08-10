#!/bin/bash
# Capsem Doctor -- development environment health check
# Usage: scripts/doctor-common.sh [--fix]
#   --fix  Auto-fix all fixable issues without prompting
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

# Share the architecture-consistency parser with canonical bootstrap. Sourcing
# the library is inert: it only defines functions and performs no Linux setup.
# shellcheck disable=SC1091
source "$SCRIPT_DIR/bootstrap-linux.sh"
# shellcheck disable=SC1091
. "$SCRIPT_DIR/bootstrap-rust.sh"
CAPSEM_RUST_TOOLCHAIN=$(capsem_rust_toolchain "$PROJECT_ROOT/rust-toolchain.toml")

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

_doctor_build_assets_all_profiles() {
    # Never from inside a gate. `just _build-assets` is `capsem-gate
    # build-assets`, which takes the machine lock -- and a gate run is already
    # holding it, so the child waits out its full timeout for a lock that
    # cannot be released until the child returns. The Python side refuses this
    # explicitly; reaching it through a shell auto-fix is the same deadlock by
    # a route that guard cannot see.
    #
    # It is also unnecessary. A run that gets here has `assets.build.<arch>`
    # further down its own plan, ordered against everything that needs the
    # result. Self-healing here would build them twice at best.
    #
    # Only visible once a run stopped inheriting a warm checkout: `assets/` is
    # build output, so a private copy has none, doctor found them missing and
    # tried to fix it mid-gate.
    if [ -n "${CAPSEM_GATE_RUN:-}" ]; then
        printf "  [SKIP] asset build (inside %s; its plan builds them)\n" "$CAPSEM_GATE_RUN"
        return 0
    fi
    local arch
    arch="$(uname -m | sed 's/aarch64/arm64/;s/arm64/arm64/;s/x86_64/x86_64/')"
    local profile
    for profile in config/profiles/*/profile.toml; do
        just _build-assets "$(basename "$(dirname "$profile")")" "$arch"
    done
}

_doctor_pack_initrd() {
    # Same deadlock, same reason: `just _pack-initrd` is `capsem-gate
    # pack-initrd`, and a gate run holds the lock it would ask for. The plan
    # that got here owns `initrd.repack`, ordered against what needs it.
    if [ -n "${CAPSEM_GATE_RUN:-}" ]; then
        printf "  [SKIP] initrd repack (inside %s; its plan does this)\n" "$CAPSEM_GATE_RUN"
        return 0
    fi
    just _pack-initrd
}

_doctor_install_node_workspaces() {
    if [ -n "${CAPSEM_GATE_RUN:-}" ]; then
        printf "  [SKIP] Node workspace install (inside %s; its plan owns it)\n" \
            "$CAPSEM_GATE_RUN"
        return 0
    fi
    uv run capsem-gate install-node
}

_doctor_install_gate_tools() {
    if [ -n "${CAPSEM_GATE_RUN:-}" ]; then
        printf "  [FAIL] Cargo gate tools must be installed before entering %s\n" \
            "$CAPSEM_GATE_RUN" >&2
        printf "         Run ./bootstrap.sh on the host, then retry the gate.\n" >&2
        return 1
    fi
    if [[ "$(uname -s)" == "Linux" ]]; then
        local rustup_bin cargo_bin
        rustup_bin=$(readlink -f "$(command -v rustup)")
        cargo_bin=$(dirname "$rustup_bin")
        PATH="$cargo_bin:$PATH" uv run capsem-gate install-tools
        capsem_expose_gate_cargo_tools "$PROJECT_ROOT/config/gate.toml"
    else
        uv run capsem-gate install-tools
    fi
}

# Order matters: tools before builds, builds before assets
_reg rustup-targets   "rustup target add --toolchain $CAPSEM_RUST_TOOLCHAIN aarch64-unknown-linux-musl x86_64-unknown-linux-musl" \
                      "Install Rust cross-compile targets"
_reg llvm-tools       "rustup component add --toolchain $CAPSEM_RUST_TOOLCHAIN llvm-tools" \
                      "Install llvm-tools (provides rust-lld)"
_reg linux-musl-tools "_doctor_install_linux_musl_tools" \
                      "Install Linux musl C compiler/linker (musl-tools)"
_reg gate-cargo-tools "_doctor_install_gate_tools" \
                      "Install every config-owned Cargo gate tool"
_reg entitlements     "git checkout entitlements.plist" \
                      "Restore entitlements.plist"
_reg cargo-config     "git checkout .cargo/config.toml" \
                      "Restore .cargo/config.toml"
_reg run-signed       "git checkout scripts/run_signed.sh && chmod +x scripts/run_signed.sh" \
                      "Restore scripts/run_signed.sh"
_reg run-signed-chmod "chmod +x scripts/run_signed.sh" \
                      "Make scripts/run_signed.sh executable"
_reg pnpm-install     "_doctor_install_node_workspaces" \
                      "Install every locked Node workspace"
_reg build-assets     "touch .dev-setup && CAPSEM_SKIP_ASSET_CHECK=1 _doctor_build_assets_all_profiles" \
                      "Build VM assets (kernel + rootfs)"
_reg pack-initrd      "touch .dev-setup && CAPSEM_SKIP_ASSET_CHECK=1 _doctor_pack_initrd" \
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
for tool in cargo rustup node python3 uv pnpm sqlite3 git b3sum flock zstd cpio; do
    if command -v "$tool" &>/dev/null; then
        pass "$tool"
    else
        _hint=$(tool_hint "$tool")
        fail "$tool not found -- install: $_hint"
    fi
done

if command -v node &>/dev/null; then
    _required_node_major=$(capsem_linux_node_major \
        "$PROJECT_ROOT/config/docker/image/build.toml")
    _installed_node_major=$(node -p 'process.versions.node.split(".")[0]' 2>/dev/null || true)
    if [[ "$_installed_node_major" =~ ^[0-9]+$ ]] \
        && (( _installed_node_major >= _required_node_major )); then
        pass "Node.js major $_installed_node_major (required Node.js major $_required_node_major or newer)"
    else
        fail "Node.js major ${_installed_node_major:-unknown} is below required Node.js major $_required_node_major -- run bootstrap.sh"
    fi
fi

section "Rust Toolchain"
if _doctor_rust_version=$(rustup run "$CAPSEM_RUST_TOOLCHAIN" rustc --version 2>/dev/null) \
    && [[ "$_doctor_rust_version" == "rustc $CAPSEM_RUST_TOOLCHAIN "* ]]; then
    pass "Rust $CAPSEM_RUST_TOOLCHAIN (checked-in toolchain)"
else
    fail "Rust $CAPSEM_RUST_TOOLCHAIN missing or unusable -- run ./bootstrap.sh"
fi
for target in aarch64-unknown-linux-musl x86_64-unknown-linux-musl; do
    if rustup target list --toolchain "$CAPSEM_RUST_TOOLCHAIN" --installed \
        2>/dev/null | grep -q "$target"; then
        pass "target: $target"
    else
        fixable rustup-targets "target: $target missing"
    fi
done
if rustup component list --toolchain "$CAPSEM_RUST_TOOLCHAIN" --installed \
    2>/dev/null | grep -q llvm-tools; then
    pass "component: llvm-tools"
else
    fixable llvm-tools "component: llvm-tools missing"
fi

if declare -F check_linux_musl_toolchain >/dev/null; then
    check_linux_musl_toolchain
fi

section "Cargo Tools"
_check_cargo_tool() {
    local tool="$1" expected="$2" probe="$3" actual first_line
    local -a probe_argv
    if ! command -v "$tool" &>/dev/null; then
        fixable gate-cargo-tools "$tool not found"
        return
    fi
    read -r -a probe_argv <<< "$probe"
    actual=$("${probe_argv[@]}" 2>/dev/null || true)
    first_line=${actual%%$'\n'*}
    if [[ "$actual" == "$expected"* ]]; then
        pass "$tool ($expected)"
    else
        fixable gate-cargo-tools \
            "$tool version mismatch (expected $expected, found ${first_line:-no output})"
    fi
}
while IFS=$'\t' read -r tool expected probe; do
    _check_cargo_tool "$tool" "$expected" "$probe"
done <<EOF
$(capsem_gate_cargo_tool_versions "$PROJECT_ROOT/config/gate.toml")
EOF

section "Container Tools"
if command -v docker &>/dev/null; then
    pass "docker CLI ($(docker --version 2>/dev/null | head -1))"
else
    _hint=$(tool_hint docker)
    fail "docker CLI not found -- install: $_hint"
fi

if docker info &>/dev/null; then
    pass "docker daemon (running)"
else
    _hint=$(tool_hint docker-daemon)
    fail "docker daemon not reachable -- $_hint"
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
        # v2 manifest: binaries.current holds the binary release that matches Cargo.toml.
        _manifest_ver=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("binaries",{}).get("current",""))' "$ASSETS_DIR/manifest.json" 2>/dev/null)
        if [[ "$_cargo_ver" == "$_manifest_ver" ]]; then
            pass "binary version ($_manifest_ver) matches Cargo.toml"
        else
            fixable build-assets "binary version mismatch: Cargo=$_cargo_ver, manifest.binaries.current=$_manifest_ver"
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
if [[ -z "${CAPSEM_SKIP_ASSET_CHECK:-}" ]]; then
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
else
    skip "Guest Binaries (CAPSEM_SKIP_ASSET_CHECK set)"
fi

section "Release Tools"
for tool in gh openssl cdxgen; do
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
                    echo -e "  ${RED}failed -- stopping (later fixes depend on this)${NC}"
                    exit 1
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
