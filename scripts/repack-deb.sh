#!/bin/bash
# repack-deb.sh -- Repack a Tauri .deb to include companion binaries and a postinst script.
#
# Usage: repack-deb.sh [--manifest manifest.json] <input.deb> <bin_dir> <config_root> [assets_dir] [output.deb]
#
# Arguments:
#   input.deb   Path to the Tauri-built .deb package
#   bin_dir     Directory containing companion binaries (capsem, capsem-service, etc.)
#   config_root Materialized runtime config root (usually target/config)
#   assets_dir  Optional assets dir. When CAPSEM_DEB_ASSET_MODE=current-arch,
#               current-arch assets are added to /usr/share/capsem/assets.
#   output.deb  Optional output path (defaults to overwriting input)
#   --manifest  Optional manifest to package instead of <assets_dir>/manifest.json.
#
# Adds to the .deb:
#   /usr/bin/capsem
#   /usr/bin/capsem-service
#   /usr/bin/capsem-process
#   /usr/bin/capsem-tui
#   /usr/bin/capsem-mcp
#   /usr/bin/capsem-gateway
#   /usr/bin/capsem-tray
#   /usr/bin/capsem-admin
#   /usr/share/capsem/profiles/
#   DEBIAN/postinst script
set -euo pipefail

usage() {
    echo "usage: repack-deb.sh [--manifest manifest.json] <input.deb> <bin_dir> <config_root> [assets_dir] [output.deb]" >&2
}

MANIFEST_PATH=""
POSITIONAL=()
while [ "$#" -gt 0 ]; do
    case "$1" in
        --manifest)
            MANIFEST_PATH="${2:?--manifest requires a path}"
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        --)
            shift
            while [ "$#" -gt 0 ]; do
                POSITIONAL+=("$1")
                shift
            done
            ;;
        --*)
            echo "ERROR: unknown option $1" >&2
            usage
            exit 2
            ;;
        *)
            POSITIONAL+=("$1")
            shift
            ;;
    esac
done

if [ "${#POSITIONAL[@]}" -lt 3 ] || [ "${#POSITIONAL[@]}" -gt 5 ]; then
    usage
    exit 2
fi

INPUT_DEB="${POSITIONAL[0]}"
BIN_DIR="${POSITIONAL[1]}"
CONFIG_ROOT="${POSITIONAL[2]}"
ASSETS_DIR="${POSITIONAL[3]:-}"
OUTPUT_DEB="${POSITIONAL[4]:-$INPUT_DEB}"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
WORK_DIR=$(mktemp -d)
trap 'rm -rf "$WORK_DIR"' EXIT

echo "=== Extracting .deb ==="
dpkg-deb -R "$INPUT_DEB" "$WORK_DIR/deb"

echo "=== Adding companion binaries ==="
mkdir -p "$WORK_DIR/deb/usr/bin"
for bin in capsem capsem-service capsem-process capsem-tui capsem-mcp capsem-mcp-aggregator capsem-mcp-builtin capsem-gateway capsem-tray capsem-admin; do
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

if [ ! -d "$CONFIG_ROOT/profiles" ]; then
    echo "ERROR: materialized profiles not found: $CONFIG_ROOT/profiles" >&2
    echo "Run: just _materialize-config" >&2
    exit 1
fi
echo "=== Adding materialized profiles ==="
mkdir -p "$WORK_DIR/deb/usr/share/capsem/profiles"
cp -R "$CONFIG_ROOT/profiles/." "$WORK_DIR/deb/usr/share/capsem/profiles/"

ASSET_MODE="${CAPSEM_DEB_ASSET_MODE:-manifest-only}"
ASSETS_VIEW="$ASSETS_DIR"
if [ -n "$MANIFEST_PATH" ]; then
    if [ ! -f "$MANIFEST_PATH" ]; then
        echo "ERROR: manifest not found: $MANIFEST_PATH" >&2
        exit 1
    fi
    ASSETS_VIEW="$WORK_DIR/assets-view"
    mkdir -p "$ASSETS_VIEW"
    cp "$MANIFEST_PATH" "$ASSETS_VIEW/manifest.json"
    if [ -n "$ASSETS_DIR" ]; then
        for arch_dir in "$ASSETS_DIR"/*; do
            [ -d "$arch_dir" ] || continue
            ln -s "$arch_dir" "$ASSETS_VIEW/$(basename "$arch_dir")"
        done
    fi
fi
if [ "$ASSET_MODE" = "current-arch" ]; then
    if [ -z "$ASSETS_VIEW" ]; then
        echo "ERROR: CAPSEM_DEB_ASSET_MODE=current-arch requires assets_dir" >&2
        exit 1
    fi
    echo "=== Adding current-arch assets ==="
    bash "$SCRIPT_DIR/sync-dev-assets.sh" "$ASSETS_VIEW" "$WORK_DIR/deb/usr/share/capsem/assets"
elif [ "$ASSET_MODE" != "manifest-only" ]; then
    echo "ERROR: unknown CAPSEM_DEB_ASSET_MODE=$ASSET_MODE" >&2
    exit 1
else
    # The selected manifest is package payload. deb-postinst copies it from
    # /usr/share/capsem/assets/manifest.json into ~/.capsem/assets/manifest.json,
    # and the daemon resolves profile assets from that installed manifest.
    if [ -n "$ASSETS_VIEW" ] && [ -f "$ASSETS_VIEW/manifest.json" ]; then
        mkdir -p "$WORK_DIR/deb/usr/share/capsem/assets"
        cp "$ASSETS_VIEW/manifest.json" "$WORK_DIR/deb/usr/share/capsem/assets/manifest.json"
    fi
fi

echo "=== Repacking .deb ==="
dpkg-deb -b "$WORK_DIR/deb" "$OUTPUT_DEB"

echo "=== Validating ==="
dpkg-deb --info "$OUTPUT_DEB"

echo "=== Built: $OUTPUT_DEB ==="
ls -lh "$OUTPUT_DEB"
