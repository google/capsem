#!/bin/bash
# repack-deb.sh -- Repack a Tauri .deb with companion binaries and preinst plus postinst scripts.
#
# Usage: repack-deb.sh [--manifest file://...|http://...|https://...] <input.deb> <bin_dir> <config_root> [assets_dir] [output.deb]
#
# Arguments:
#   input.deb   Path to the Tauri-built .deb package
#   bin_dir     Directory containing companion binaries (capsem, capsem-service, etc.)
#   config_root Materialized runtime config root (usually target/config)
#   assets_dir  Optional assets dir containing manifest.json when --manifest is omitted.
#   output.deb  Optional output path (defaults to overwriting input)
#   --manifest  Optional manifest URL to package instead of <assets_dir>/manifest.json.
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
#   DEBIAN/preinst script
#   DEBIAN/postinst script
set -euo pipefail
export COPYFILE_DISABLE=1

usage() {
    echo "usage: repack-deb.sh [--manifest file://...|http://...|https://...] <input.deb> <bin_dir> <config_root> [assets_dir] [output.deb]" >&2
}

MANIFEST_PATH=""
POSITIONAL=()
while [ "$#" -gt 0 ]; do
    case "$1" in
        --manifest)
            MANIFEST_PATH="${2:?--manifest requires a URL}"
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

write_manifest_origin() {
    local manifest_source="${1:?write_manifest_origin <manifest_source> <manifest_path> <package_version> <dst>}"
    local manifest_path="${2:?write_manifest_origin <manifest_source> <manifest_path> <package_version> <dst>}"
    local package_version="${3:?write_manifest_origin <manifest_source> <manifest_path> <package_version> <dst>}"
    local dst="${4:?write_manifest_origin <manifest_source> <manifest_path> <package_version> <dst>}"
    python3 - "$manifest_source" "$manifest_path" "$package_version" "$dst" <<'PY'
import datetime
import hashlib
import json
import pathlib
import sys
import urllib.parse
import urllib.request

raw_source = sys.argv[1]
manifest_path = pathlib.Path(sys.argv[2])
package_version = sys.argv[3]
dst = pathlib.Path(sys.argv[4])
parsed = urllib.parse.urlparse(raw_source)
if parsed.scheme not in ("http", "https", "file"):
    raise SystemExit(f"manifest source must be a URL: {raw_source}")
manifest_bytes = manifest_path.read_bytes()
fetched_at = datetime.datetime.now(datetime.timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")
dst.write_text(json.dumps({
    "schema": "capsem.manifest_origin.v1",
    "origin": "package",
    "source": raw_source,
    "fetched_at": fetched_at,
    "packaged_at": fetched_at,
    "package_version": package_version,
    "snapshot_sha256": hashlib.sha256(manifest_bytes).hexdigest(),
}, sort_keys=True) + "\n")
PY
}

file_url() {
    local path="${1:?file_url <path>}"
    python3 - "$path" <<'PY'
import pathlib
import sys

print(pathlib.Path(sys.argv[1]).resolve().as_uri())
PY
}

materialize_manifest_input() {
    local manifest_source="${1:?materialize_manifest_input <manifest_source> <dst>}"
    local dst="${2:?materialize_manifest_input <manifest_source> <dst>}"
    python3 - "$manifest_source" "$dst" <<'PY'
import pathlib
import sys
import urllib.parse
import urllib.request

source = sys.argv[1]
dst = pathlib.Path(sys.argv[2])
parsed = urllib.parse.urlparse(source)

if parsed.scheme in ("http", "https"):
    request = urllib.request.Request(
        source,
        headers={"User-Agent": "CapsemReleaseValidator/1.0"},
    )
    with urllib.request.urlopen(request, timeout=60) as response:
        dst.write_bytes(response.read())
elif parsed.scheme == "file":
    dst.write_bytes(pathlib.Path(urllib.request.url2pathname(parsed.path)).read_bytes())
elif parsed.scheme:
    raise SystemExit(f"unsupported manifest URL scheme: {parsed.scheme}")
else:
    raise SystemExit(f"manifest must be a URL: use https://..., http://..., or file:///absolute/path, got {source}")
PY
}

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

echo "=== Adding maintainer scripts ==="
cp "$SCRIPT_DIR/deb-preinst.sh" "$WORK_DIR/deb/DEBIAN/preinst"
chmod 755 "$WORK_DIR/deb/DEBIAN/preinst"
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

ASSETS_VIEW="$ASSETS_DIR"
SELECTED_MANIFEST_SOURCE="$(file_url "$ASSETS_DIR/manifest.json")"
if [ -n "$MANIFEST_PATH" ]; then
    SELECTED_MANIFEST_SOURCE="$MANIFEST_PATH"
    ASSETS_VIEW="$WORK_DIR/assets-view"
    mkdir -p "$ASSETS_VIEW"
    materialize_manifest_input "$MANIFEST_PATH" "$ASSETS_VIEW/manifest.json"
fi
if [ -z "$ASSETS_VIEW" ] || [ ! -f "$ASSETS_VIEW/manifest.json" ]; then
    echo "ERROR: manifest not found: $ASSETS_VIEW/manifest.json" >&2
    exit 1
fi
mkdir -p "$WORK_DIR/deb/usr/share/capsem/assets"
cp "$ASSETS_VIEW/manifest.json" "$WORK_DIR/deb/usr/share/capsem/assets/manifest.json"
PACKAGE_VERSION="$(dpkg-deb -f "$INPUT_DEB" Version)"
write_manifest_origin "$SELECTED_MANIFEST_SOURCE" "$WORK_DIR/deb/usr/share/capsem/assets/manifest.json" "$PACKAGE_VERSION" "$WORK_DIR/deb/usr/share/capsem/assets/manifest-origin.json"

echo "=== Repacking .deb ==="
dpkg-deb -b "$WORK_DIR/deb" "$OUTPUT_DEB"

echo "=== Validating ==="
dpkg-deb --info "$OUTPUT_DEB"

echo "=== Built: $OUTPUT_DEB ==="
ls -lh "$OUTPUT_DEB"
