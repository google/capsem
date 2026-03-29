#!/bin/bash
# Capsem developer bootstrap -- checks tools, installs deps, runs doctor.
# Usage: bash scripts/bootstrap.sh
set -euo pipefail

PASS=0; FAIL=0; MISSING=()

pass() { echo "  [ok]   $1"; PASS=$((PASS + 1)); }
miss() { echo "  [MISS] $1 -- $2"; FAIL=$((FAIL + 1)); MISSING+=("$1"); }

echo "Capsem Bootstrap"
echo "================"
echo ""

# --- Phase 1: Core tools ---
echo "== Checking tools =="

if command -v rustup &>/dev/null; then pass "rustup"; else miss "rustup" "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"; fi
if command -v cargo &>/dev/null; then pass "cargo"; else miss "cargo" "installed with rustup"; fi
if command -v just &>/dev/null; then pass "just"; else miss "just" "cargo install just"; fi
if command -v node &>/dev/null; then pass "node ($(node --version))"; else miss "node" "brew install node (24+ required)"; fi
if command -v pnpm &>/dev/null; then pass "pnpm"; else miss "pnpm" "npm i -g pnpm"; fi
if command -v python3 &>/dev/null; then pass "python3"; else miss "python3" "brew install python"; fi
if command -v uv &>/dev/null; then pass "uv"; else miss "uv" "curl -LsSf https://astral.sh/uv/install.sh | sh"; fi
if command -v git &>/dev/null; then pass "git"; else miss "git" "brew install git"; fi

# Container runtime
if command -v docker &>/dev/null; then
    pass "docker"
elif command -v podman &>/dev/null; then
    pass "podman"
else
    miss "docker/podman" "brew install podman && podman machine init --memory 8192 --cpus 8 && podman machine start"
fi

echo ""
echo "== Results: $PASS found, $FAIL missing =="

if [ "$FAIL" -gt 0 ]; then
    echo ""
    echo "Install the missing tools above, then re-run this script."
    exit 1
fi

# --- Phase 2: Install dependencies ---
echo ""
echo "== Installing dependencies =="

echo "  Python deps (uv sync)..."
uv sync --quiet

echo "  Frontend deps (pnpm install)..."
cd frontend && pnpm install --frozen-lockfile --silent && cd ..

# --- Phase 3: Run doctor ---
echo ""
echo "== Running just doctor =="
echo ""
just doctor

echo ""
echo "================"
echo "Bootstrap complete. Next steps:"
echo ""
echo "  just build-assets    # Build VM kernel + rootfs (~10 min, needs Docker/Podman)"
echo "  just run \"echo hi\"   # Verify VM boots"
echo ""
