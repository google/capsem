#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
OUTPUT_DIR=${CAPSEM_LINUX_RUST_OUTPUT_DIR:-$ROOT/cache/target/coverage/linux}
mkdir -p "$OUTPUT_DIR"
export NEXTEST_STATE_DIR="$OUTPUT_DIR/nextest"

cd "$ROOT"

# The macOS-hosted sealed lane inherits these dependency trees from its
# networked base. Native Linux CI starts from a clean checkout and installs
# them here before rebuilding the current source.
if [[ ! -d "$ROOT/sdk/typescript/node_modules" ]]; then
    pnpm --dir sdk/typescript install --frozen-lockfile
fi
if [[ ! -d "$ROOT/web/app/node_modules" ]]; then
    pnpm --dir web/app install --frozen-lockfile
fi
pnpm --dir sdk/typescript run build
bash build_system/scripts/web/check-web-surface.sh frontend-build
test -s "$ROOT/web/app/dist/index.html"

cross_target=$(python3 build_system/scripts/bootstrap/provision-linux-workspace.py --cross-rust-target)
cargo clippy --target "$cross_target" -p capsem-core --lib --tests -- -D warnings
cargo clippy --workspace --all-targets -- -D warnings

# The OS confinement boundary is exercised by an actual executable child.
cargo nextest run --locked -p capsem-router --test subprocess --profile ci

# Service unit tests discover and execute the real native MCP cohort. A
# coverage test executable is not the production worker it supervises. Build
# beside the coverage test cohort, since nextest runs from each package root.
cargo llvm-cov clean --workspace
cargo llvm-cov run --locked --no-cfg-coverage --no-report -p capsem-mcp-aggregator -- --version
cargo llvm-cov run --locked --no-cfg-coverage --no-report -p capsem-mcp-builtin -- --version

uv run --project build_system --frozen python build_system/scripts/test/linux-rust-coverage.py
