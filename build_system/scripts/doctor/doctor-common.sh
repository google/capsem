#!/bin/bash
# Capsem Doctor -- development environment health check
# Usage: build_system/scripts/doctor/doctor-common.sh [--fix]
#   --fix  Auto-fix all fixable issues without prompting
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
cd "$PROJECT_ROOT"

# Share the architecture-consistency parser with canonical bootstrap. Sourcing
# the library is inert: it only defines functions and performs no Linux setup.
# shellcheck disable=SC1091
source "$PROJECT_ROOT/build_system/scripts/bootstrap/bootstrap-linux.sh"
# shellcheck disable=SC1091
. "$PROJECT_ROOT/build_system/scripts/bootstrap/bootstrap-rust.sh"
CAPSEM_RUST_TOOLCHAIN=$(capsem_rust_toolchain "$PROJECT_ROOT/rust-toolchain.toml")

# These are consumed by the sourced platform and check fragments. ShellCheck
# evaluates this coordinator as a standalone file in the repository-wide pass.
# shellcheck disable=SC2034
ENTITLEMENTS="build_system/packaging/macos/entitlements.plist"
# shellcheck disable=SC2034
ASSETS_DIR="target/assets"

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
    uv run --project build_system --frozen capsem-gate install-node
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
        PATH="$cargo_bin:$PATH" uv run --project build_system --frozen capsem-gate install-tools
        capsem_expose_gate_cargo_tools "$PROJECT_ROOT/config/gate.toml"
    else
        uv run --project build_system --frozen capsem-gate install-tools
    fi
}

# Order matters: tools before builds, builds before assets
_reg rustup-targets   "capsem_install_rust_targets $CAPSEM_RUST_TOOLCHAIN $PROJECT_ROOT/config/gate.toml" \
                      "Install Rust cross-compile targets"
_reg llvm-tools       "rustup component add --toolchain $CAPSEM_RUST_TOOLCHAIN llvm-tools" \
                      "Install llvm-tools (provides rust-lld)"
_reg linux-musl-tools "_doctor_install_linux_musl_tools" \
                      "Install Linux musl C compiler/linker (musl-tools)"
_reg gate-cargo-tools "_doctor_install_gate_tools" \
                      "Install every config-owned Cargo gate tool"
_reg entitlements     "git checkout build_system/packaging/macos/entitlements.plist" \
                      "Restore entitlements.plist"
_reg cargo-config     "git checkout .cargo/config.toml" \
                      "Restore .cargo/config.toml"
_reg run-signed       "git checkout build_system/packaging/macos/run_signed.sh && chmod +x build_system/packaging/macos/run_signed.sh" \
                      "Restore build_system/packaging/macos/run_signed.sh"
_reg run-signed-chmod "chmod +x build_system/packaging/macos/run_signed.sh" \
                      "Make build_system/packaging/macos/run_signed.sh executable"
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

# Execute the checks in the same sourced shell so the registry, counters,
# platform functions, and original command-line arguments remain shared.
# shellcheck source=doctor-run.sh
source "$SCRIPT_DIR/doctor-run.sh"
