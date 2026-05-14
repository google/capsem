#!/bin/bash
# build-assets.sh -- Build guest VM assets and regenerate checksums/manifest.
#
# Usage:
#   scripts/build-assets.sh [--assets-dir assets] [--arch arm64|x86_64]
#
# If --arch is omitted, both arm64 and x86_64 are rebuilt.
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: scripts/build-assets.sh [--assets-dir <dir>] [--arch <arch>]

Options:
  --assets-dir <dir>   Assets output directory (default: assets)
  --arch <arch>        One arch to rebuild (arm64|aarch64|x86_64|amd64)
  -h, --help           Show this help
EOF
}

normalize_arch() {
    case "$1" in
        arm64|aarch64) echo "arm64" ;;
        x86_64|amd64) echo "x86_64" ;;
        *)
            echo "ERROR: unsupported arch '$1' (expected arm64 or x86_64)" >&2
            exit 1
            ;;
    esac
}

ASSETS_DIR="assets"
ARCH=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --assets-dir)
            ASSETS_DIR="${2:?missing value for --assets-dir}"
            shift 2
            ;;
        --arch)
            ARCH="${2:?missing value for --arch}"
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "ERROR: unknown argument '$1'" >&2
            usage
            exit 1
            ;;
    esac
done

if [[ -n "$ARCH" ]]; then
    ARCH="$(normalize_arch "$ARCH")"
    arches=("$ARCH")
    echo "=== Cleaning assets for $ARCH ==="
    rm -rf "$ASSETS_DIR/$ARCH"
else
    arches=(arm64 x86_64)
    echo "=== Cleaning all assets ==="
    rm -rf "$ASSETS_DIR/arm64" "$ASSETS_DIR/x86_64"
    rm -f "$ASSETS_DIR/manifest.json" "$ASSETS_DIR/B3SUMS"
fi

for arch_name in "${arches[@]}"; do
    echo "=== Building kernel for $arch_name ==="
    uv run capsem-builder build guest/ --arch "$arch_name" --template kernel --output "$ASSETS_DIR/"
    echo
    echo "=== Building rootfs for $arch_name ==="
    uv run capsem-builder build guest/ --arch "$arch_name" --template rootfs --output "$ASSETS_DIR/"
    echo
done

echo "=== Generating checksums ==="
uv run python3 - "$ASSETS_DIR" <<'PY'
from pathlib import Path
import sys

from capsem.builder.docker import generate_checksums, get_project_version

assets_dir = Path(sys.argv[1])
version = get_project_version(Path("."))
generate_checksums(assets_dir, version)
print(f"manifest.json generated (v{version})")
PY
