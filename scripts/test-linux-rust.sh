#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
OUTPUT_DIR=${CAPSEM_LINUX_RUST_OUTPUT_DIR:-$ROOT}
mkdir -p "$OUTPUT_DIR"

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
cargo llvm-cov nextest \
    --no-cfg-coverage \
    --bins \
    --profile ci \
    --codecov \
    --output-path "$OUTPUT_DIR/codecov-linux.json" \
    "${package_args[@]}"

set -o pipefail
cargo llvm-cov report \
    --summary-only \
    "${package_args[@]}" \
    2>&1 | tee "$OUTPUT_DIR/coverage-summary-linux.txt"
