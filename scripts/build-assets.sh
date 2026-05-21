#!/bin/bash
# build-assets.sh -- Build guest VM assets and regenerate checksums/manifest.
#
# Usage:
#   scripts/build-assets.sh [--assets-dir assets] [--arch arm64|x86_64] [--profile profile.toml]
#
# If --arch is omitted, both arm64 and x86_64 are rebuilt.
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: scripts/build-assets.sh [--assets-dir <dir>] [--arch <arch>] [--profile <profile>]

Options:
  --assets-dir <dir>   Assets output directory (default: assets)
  --arch <arch>        One arch to rebuild (arm64|aarch64|x86_64|amd64)
  --profile <profile>  Profile V2 JSON/TOML payload to drive image builds
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
PROFILE=""

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
        --profile)
            PROFILE="${2:?missing value for --profile}"
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

if [[ -n "$PROFILE" && ! -f "$PROFILE" ]]; then
    echo "ERROR: profile '$PROFILE' does not exist" >&2
    exit 1
fi

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

build_template() {
    local arch_name="$1"
    local template="$2"

    if [[ -n "$PROFILE" ]]; then
        uv run capsem-admin image build "$PROFILE" \
            --arch "$arch_name" \
            --template "$template" \
            --out "$ASSETS_DIR/" \
            --json
    else
        uv run capsem-builder build guest/ \
            --arch "$arch_name" \
            --template "$template" \
            --output "$ASSETS_DIR/"
    fi
}

for arch_name in "${arches[@]}"; do
    echo "=== Building kernel for $arch_name ==="
    build_template "$arch_name" kernel
    echo
    echo "=== Building rootfs for $arch_name ==="
    build_template "$arch_name" rootfs
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
