#!/bin/bash
# repack-deb.sh -- Repack a Tauri .deb to include companion binaries and a postinst script.
#
# Usage: repack-deb.sh <input.deb> <bin_dir> [assets_dir] [output.deb]
#
# Arguments:
#   input.deb   Path to the Tauri-built .deb package
#   bin_dir     Directory containing companion binaries (capsem, capsem-service, etc.)
#   assets_dir  Optional directory containing manifest.json + manifest.json.minisig
#   output.deb  Optional output path (defaults to overwriting input)
#
# Adds to the .deb:
#   /usr/bin/capsem
#   /usr/bin/capsem-service
#   /usr/bin/capsem-process
#   /usr/bin/capsem-mcp
#   /usr/bin/capsem-mcp-aggregator
#   /usr/bin/capsem-mcp-builtin
#   /usr/bin/capsem-gateway
#   /usr/bin/capsem-tray
#   /usr/share/capsem/assets/manifest.json{,.minisig} when assets_dir is provided
#   DEBIAN/postinst script
set -euo pipefail

INPUT_DEB="${1:?usage: repack-deb.sh <input.deb> <bin_dir> [assets_dir] [output.deb]}"
BIN_DIR="${2:?usage: repack-deb.sh <input.deb> <bin_dir> [assets_dir] [output.deb]}"
ASSETS_DIR=""
OUTPUT_DEB="$INPUT_DEB"
if [ "${3:-}" != "" ]; then
    if [ -d "$3" ]; then
        ASSETS_DIR="$3"
        OUTPUT_DEB="${4:-$INPUT_DEB}"
    elif [ "${4:-}" != "" ]; then
        echo "ERROR: assets_dir is not a directory: $3" >&2
        exit 1
    elif [[ "$3" == *.deb ]]; then
        OUTPUT_DEB="$3"
    else
        echo "ERROR: third argument is neither an existing assets directory nor a .deb output path: $3" >&2
        echo "       Usage: repack-deb.sh <input.deb> <bin_dir> [assets_dir] [output.deb]" >&2
        exit 1
    fi
fi

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
WORK_DIR=$(mktemp -d)
trap 'rm -rf "$WORK_DIR"' EXIT

echo "=== Extracting .deb ==="
dpkg-deb -R "$INPUT_DEB" "$WORK_DIR/deb"

echo "=== Adding companion binaries ==="
mkdir -p "$WORK_DIR/deb/usr/bin"
for bin in capsem capsem-service capsem-process capsem-mcp capsem-mcp-aggregator capsem-mcp-builtin capsem-gateway capsem-tray; do
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

if [ -n "$ASSETS_DIR" ]; then
    echo "=== Adding signed manifest ==="
    mkdir -p "$WORK_DIR/deb/usr/share/capsem/assets"
    for asset in manifest.json manifest.json.minisig; do
        src="$ASSETS_DIR/$asset"
        if [ -f "$src" ]; then
            cp "$src" "$WORK_DIR/deb/usr/share/capsem/assets/$asset"
            chmod 644 "$WORK_DIR/deb/usr/share/capsem/assets/$asset"
            echo "  Added: $asset"
        else
            echo "  ERROR: signed manifest file not found: $src" >&2
            exit 1
        fi
    done
    if [ -f "$ASSETS_DIR/manifest-sign.dev.pub" ]; then
        cp "$ASSETS_DIR/manifest-sign.dev.pub" "$WORK_DIR/deb/usr/share/capsem/assets/manifest-sign.dev.pub"
        chmod 644 "$WORK_DIR/deb/usr/share/capsem/assets/manifest-sign.dev.pub"
        echo "  Added: manifest-sign.dev.pub"
    fi
fi

echo "=== Adding postinst script ==="
cp "$SCRIPT_DIR/deb-postinst.sh" "$WORK_DIR/deb/DEBIAN/postinst"
chmod 755 "$WORK_DIR/deb/DEBIAN/postinst"

# Stamp build timestamp into version so each build is seen as newer
BUILD_TS=$(date +%s)
CONTROL_FILE="$WORK_DIR/deb/DEBIAN/control"
CONTROL_TMP=$(mktemp)
sed "s/^Version: \(.*\)/Version: \1.$BUILD_TS/" "$CONTROL_FILE" > "$CONTROL_TMP"
mv "$CONTROL_TMP" "$CONTROL_FILE"

echo "=== Repacking .deb ==="
dpkg-deb -b "$WORK_DIR/deb" "$OUTPUT_DEB"

echo "=== Validating ==="
dpkg-deb --info "$OUTPUT_DEB"

echo "=== Built: $OUTPUT_DEB ==="
ls -lh "$OUTPUT_DEB"
