#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
OUTPUT_DIR=${CAPSEM_LINUX_RUST_OUTPUT_DIR:-$ROOT/cache/target/coverage/linux}
mkdir -p "$OUTPUT_DIR"
export NEXTEST_STATE_DIR="$OUTPUT_DIR/nextest"

packages=(
    capsem-assets
    capsem-api
    capsem-sdk
    capsem-config
    capsem-credentials
    capsem-foundation
    capsem-core
    capsem-admin
    capsem-agent
    capsem-logger
    capsem-proto
    capsem-guard
    capsem-gateway
    capsem-service
    capsem
    capsem-tui
    capsem-mcp-aggregator
    capsem-mcp-builtin
    capsem-process
    capsem-router
    capsem-bench
    capsem-mock-server
)

package_args=()
for package in "${packages[@]}"; do
    package_args+=( -p "$package" )
done

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

cargo llvm-cov nextest \
    --no-cfg-coverage \
    --lib \
    --bins \
    --profile ci \
    --codecov \
    --output-path "$OUTPUT_DIR/codecov.json" \
    "${package_args[@]}"

set -o pipefail
cargo llvm-cov report \
    --summary-only \
    "${package_args[@]}" \
    2>&1 | tee "$OUTPUT_DIR/summary.txt"
