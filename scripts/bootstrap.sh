#!/bin/sh
# Capsem developer bootstrap -- installs deps, runs doctor with auto-fix.
# Only prerequisite: sh, git, curl.
# Usage: sh scripts/bootstrap.sh
set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

printf "Capsem Bootstrap (%s)\n" "$(uname -s)"
echo "========================"
echo ""

# --- Phase 1: Bare minimum tools ---
MISSING=0
for tool in bash git curl; do
    if command -v "$tool" >/dev/null 2>&1; then
        printf "  [ok]   %s\n" "$tool"
    else
        printf "  [MISS] %s\n" "$tool"
        MISSING=$((MISSING + 1))
    fi
done

# Check for rustup/cargo (needed for cargo tools)
if ! command -v rustup >/dev/null 2>&1; then
    printf "  [MISS] rustup -- install: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh\n"
    MISSING=$((MISSING + 1))
fi

# Check for just (needed for everything else)
if ! command -v just >/dev/null 2>&1; then
    printf "  [MISS] just -- install: cargo install just\n"
    MISSING=$((MISSING + 1))
fi

if [ "$MISSING" -gt 0 ]; then
    echo ""
    echo "Install the missing tools above, then re-run: sh scripts/bootstrap.sh"
    exit 1
fi

# --- Phase 2: Install dependencies ---
echo ""
echo "== Installing dependencies =="

if command -v uv >/dev/null 2>&1; then
    printf "  Python deps (uv sync)...\n"
    uv sync --quiet
else
    printf "  [SKIP] Python deps (uv not installed yet -- doctor will catch this)\n"
fi

if command -v pnpm >/dev/null 2>&1; then
    printf "  Frontend deps (pnpm install)...\n"
    (cd frontend && pnpm install --frozen-lockfile --silent)
else
    printf "  [SKIP] Frontend deps (pnpm not installed yet -- doctor will catch this)\n"
fi

# --- Phase 3: Run doctor with auto-fix ---
echo ""
echo "== Running doctor (with auto-fix) =="
echo ""
"$SCRIPT_DIR/doctor-common.sh" --fix

echo ""
echo "========================"
echo "Bootstrap complete. Next steps:"
echo ""
echo "  just build-assets    # Build VM kernel + rootfs (~10 min, needs Docker)"
echo "  just exec \"echo hi\"  # Verify VM boots"
echo ""
