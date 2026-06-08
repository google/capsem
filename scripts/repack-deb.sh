#!/bin/bash
# repack-deb.sh -- Repack a Tauri .deb to include companion binaries and a postinst script.
#
# Usage: repack-deb.sh <input.deb> <bin_dir> [assets_dir] [output.deb]
#
# Arguments:
#   input.deb   Path to the Tauri-built .deb package
#   bin_dir     Directory containing companion binaries (capsem, capsem-service, etc.)
#   assets_dir  Optional assets dir. When CAPSEM_DEB_ASSET_MODE=current-arch,
#               current-arch assets are added to /usr/share/capsem/assets.
#   output.deb  Optional output path (defaults to overwriting input)
#
# Adds to the .deb:
#   /usr/bin/capsem
#   /usr/bin/capsem-service
#   /usr/bin/capsem-process
#   /usr/bin/capsem-mcp
#   /usr/bin/capsem-gateway
#   /usr/bin/capsem-tray
#   /usr/bin/capsem-admin
#   DEBIAN/postinst script
set -euo pipefail

INPUT_DEB="${1:?usage: repack-deb.sh <input.deb> <bin_dir> [assets_dir] [output.deb]}"
BIN_DIR="${2:?usage: repack-deb.sh <input.deb> <bin_dir> [assets_dir] [output.deb]}"
ASSETS_DIR="${3:-}"
OUTPUT_DEB="${4:-$INPUT_DEB}"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
WORK_DIR=$(mktemp -d)
trap 'rm -rf "$WORK_DIR"' EXIT

echo "=== Extracting .deb ==="
dpkg-deb -R "$INPUT_DEB" "$WORK_DIR/deb"

echo "=== Adding companion binaries ==="
mkdir -p "$WORK_DIR/deb/usr/bin"
for bin in capsem capsem-service capsem-process capsem-mcp capsem-mcp-aggregator capsem-mcp-builtin capsem-gateway capsem-tray capsem-admin; do
    src="$BIN_DIR/$bin"
    if [ -f "$src" ]; then
        cp "$src" "$WORK_DIR/deb/usr/bin/$bin"
        chmod 755 "$WORK_DIR/deb/usr/bin/$bin"
        echo "  Added: $bin"
    else
        echo "  ERROR: binary not found: $src" >&2
        exit 1
    fi
done

echo "=== Adding postinst script ==="
cp "$SCRIPT_DIR/deb-postinst.sh" "$WORK_DIR/deb/DEBIAN/postinst"
chmod 755 "$WORK_DIR/deb/DEBIAN/postinst"

ASSET_MODE="${CAPSEM_DEB_ASSET_MODE:-manifest-only}"
if [ "$ASSET_MODE" = "current-arch" ]; then
    if [ -z "$ASSETS_DIR" ]; then
        echo "ERROR: CAPSEM_DEB_ASSET_MODE=current-arch requires assets_dir" >&2
        exit 1
    fi
    echo "=== Adding current-arch assets ==="
    bash "$SCRIPT_DIR/sync-dev-assets.sh" "$ASSETS_DIR" "$WORK_DIR/deb/usr/share/capsem/assets"
elif [ "$ASSET_MODE" != "manifest-only" ]; then
    echo "ERROR: unknown CAPSEM_DEB_ASSET_MODE=$ASSET_MODE" >&2
    exit 1
fi

# Stamp build timestamp into version so each build is seen as newer
BUILD_TS=$(date +%s)
sed -i "s/^Version: \(.*\)/Version: \1.$BUILD_TS/" "$WORK_DIR/deb/DEBIAN/control"

echo "=== Repacking .deb ==="
dpkg-deb -b "$WORK_DIR/deb" "$OUTPUT_DEB"

echo "=== Validating ==="
dpkg-deb --info "$OUTPUT_DEB"

echo "=== Built: $OUTPUT_DEB ==="
ls -lh "$OUTPUT_DEB"
