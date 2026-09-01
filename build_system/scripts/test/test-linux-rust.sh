#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
OUTPUT_DIR=${CAPSEM_LINUX_RUST_OUTPUT_DIR:-$ROOT/cache/target/coverage/linux}
mkdir -p "$OUTPUT_DIR"
export NEXTEST_STATE_DIR="$OUTPUT_DIR/nextest"

packages=(
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
    capsem-mcp
    capsem-mcp-aggregator
    capsem-mcp-builtin
    capsem-process
    capsem-bench
    capsem-mock-server
)

package_args=()
for package in "${packages[@]}"; do
    package_args+=( -p "$package" )
done

cd "$ROOT"

# capsem-app embeds web/app/dist at compile time. The macOS full gate builds
# it before mounting this checkout read-only in the Linux parity container;
# the independent native-Linux CI job has to materialize it for itself.
if [[ ! -s "$ROOT/web/app/dist/index.html" ]]; then
    pnpm --dir web/app install --frozen-lockfile
    bash build_system/scripts/web/check-web-surface.sh frontend-build
fi

cross_target=$(python3 build_system/scripts/bootstrap/provision-linux-workspace.py --cross-rust-target)
cargo clippy --target "$cross_target" -p capsem-core --lib --tests -- -D warnings
cargo clippy --workspace --all-targets -- -D warnings

cargo llvm-cov nextest \
    --no-cfg-coverage \
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
