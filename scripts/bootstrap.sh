#!/bin/sh
# Capsem developer bootstrap -- checks tools, installs deps, runs doctor.
# Only prerequisite: sh. Works on macOS and Linux (apt/dnf).
# Usage: sh scripts/bootstrap.sh
set -eu

# --- OS detection ---
OS="$(uname -s)"
PKG="unknown"
if [ "$OS" = "Linux" ]; then
    if command -v apt-get >/dev/null 2>&1; then PKG="apt"
    elif command -v dnf >/dev/null 2>&1; then PKG="dnf"
    fi
fi

PASS=0
FAIL=0

pass() { printf "  [ok]   %s\n" "$1"; PASS=$((PASS + 1)); }

miss() {
    printf "  [MISS] %s\n" "$1"
    printf "         install: %s\n" "$2"
    FAIL=$((FAIL + 1))
}

# --- Platform-aware install hint ---
hint_for() {
    tool="$1"
    case "$tool" in
        rustup)
            echo "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh" ;;
        cargo)
            echo "installed with rustup (see above)" ;;
        just)
            echo "cargo install just" ;;
        node)
            case "$OS" in
                Darwin) echo "brew install node  (24+ required)" ;;
                Linux)
                    case "$PKG" in
                        apt) echo "sudo apt install nodejs npm  (24+ required, or use https://nodejs.org)" ;;
                        dnf) echo "sudo dnf install nodejs npm  (24+ required, or use https://nodejs.org)" ;;
                        *)   echo "install Node.js 24+ from https://nodejs.org" ;;
                    esac ;;
                *)  echo "install Node.js 24+ from https://nodejs.org" ;;
            esac ;;
        pnpm)
            echo "npm i -g pnpm" ;;
        python3)
            case "$OS" in
                Darwin) echo "brew install python" ;;
                Linux)
                    case "$PKG" in
                        apt) echo "sudo apt install python3 python3-venv" ;;
                        dnf) echo "sudo dnf install python3" ;;
                        *)   echo "install Python 3.11+ from https://python.org" ;;
                    esac ;;
                *)  echo "install Python 3.11+ from https://python.org" ;;
            esac ;;
        uv)
            echo "curl -LsSf https://astral.sh/uv/install.sh | sh" ;;
        git)
            case "$OS" in
                Darwin) echo "brew install git" ;;
                Linux)
                    case "$PKG" in
                        apt) echo "sudo apt install git" ;;
                        dnf) echo "sudo dnf install git" ;;
                        *)   echo "install git from https://git-scm.com" ;;
                    esac ;;
                *)  echo "install git from https://git-scm.com" ;;
            esac ;;
        docker)
            case "$OS" in
                Darwin) echo "brew install colima docker && colima start --vm-type vz --vz-rosetta --memory 8 --cpu 8" ;;
                Linux)
                    case "$PKG" in
                        apt) echo "sudo apt install docker.io" ;;
                        dnf) echo "sudo dnf install docker" ;;
                        *)   echo "install docker for your distribution" ;;
                    esac ;;
                *)  echo "install docker" ;;
            esac ;;
    esac
}

printf "Capsem Bootstrap (%s)\n" "$OS"
echo "========================"
echo ""

# --- Phase 1: Core tools ---
echo "== Checking tools =="

for tool in rustup cargo just node pnpm python3 uv git; do
    if command -v "$tool" >/dev/null 2>&1; then
        if [ "$tool" = "node" ]; then
            pass "node ($(node --version))"
        else
            pass "$tool"
        fi
    else
        miss "$tool" "$(hint_for "$tool")"
    fi
done

# Container runtime
if command -v docker >/dev/null 2>&1; then
    pass "docker"
else
    miss "docker" "$(hint_for "docker")"
fi
# Docker BuildKit (buildx) -- required for cross-arch container builds
if docker buildx version >/dev/null 2>&1; then
    pass "docker buildx"
else
    if [ "$OS" = "Darwin" ]; then
        miss "docker buildx" "brew install docker-buildx && ln -sf \$(brew --prefix docker-buildx)/bin/docker-buildx ~/.docker/cli-plugins/docker-buildx"
    else
        miss "docker buildx" "sudo apt install docker-buildx-plugin"
    fi
fi
# Colima (macOS only -- manages the container VM)
if [ "$OS" = "Darwin" ]; then
    if command -v colima >/dev/null 2>&1; then
        pass "colima"
    else
        miss "colima" "brew install colima && colima start --vm-type vz --vz-rosetta --memory 8 --cpu 8"
    fi
fi

# --- macOS: Xcode Command Line Tools + codesigning ---
if [ "$OS" = "Darwin" ]; then
    if xcode-select -p >/dev/null 2>&1; then
        pass "Xcode Command Line Tools ($(xcode-select -p))"
    else
        miss "Xcode Command Line Tools" "xcode-select --install"
    fi

    # Cargo runner config (signs binaries with Virtualization entitlements)
    if [ -f ".cargo/config.toml" ] && grep -q 'runner.*run_signed' .cargo/config.toml; then
        pass ".cargo/config.toml (cargo runner)"
    else
        miss ".cargo/config.toml" "git checkout .cargo/config.toml"
    fi
fi

# --- Linux: VM feature notice ---
if [ "$OS" = "Linux" ]; then
    printf "\n"
    printf "  [INFO] Linux uses KVM for VM features. Ensure /dev/kvm is accessible.\n"
    printf "         macOS uses Apple Virtualization.framework (codesigning required).\n"
fi

echo ""
printf "== Results: %d found, %d missing ==\n" "$PASS" "$FAIL"

if [ "$FAIL" -gt 0 ]; then
    echo ""
    echo "Install the missing tools above, then re-run: sh scripts/bootstrap.sh"
    exit 1
fi

# --- Phase 2: Install dependencies ---
echo ""
echo "== Installing dependencies =="

printf "  Python deps (uv sync)...\n"
uv sync --quiet

printf "  Frontend deps (pnpm install)...\n"
(cd frontend && pnpm install --frozen-lockfile --silent)

# --- Phase 3: Run doctor ---
echo ""
echo "== Running just doctor =="
echo ""
just doctor

echo ""
echo "========================"
echo "Bootstrap complete. Next steps:"
echo ""
echo "  just build-assets    # Build VM kernel + rootfs (~10 min, needs Docker via Colima on macOS)"
echo "  just run \"echo hi\"   # Verify VM boots"
echo ""
