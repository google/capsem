#!/bin/bash
# repack-deb.sh -- Repack a Tauri .deb to include companion binaries and a postinst script.
#
# Usage: repack-deb.sh [--manifest manifest.json|file://...|http://...|https://...] <input.deb> <bin_dir> <config_root> [assets_dir] [output.deb]
#
# Arguments:
#   input.deb   Path to the Tauri-built .deb package
#   bin_dir     Directory containing companion binaries (capsem, capsem-service, etc.)
#   config_root Materialized runtime config root (usually target/config)
#   assets_dir  Optional assets dir used only to resolve arch directories when
#               a manifest override is inspected by package tooling.
#   output.deb  Optional output path (defaults to overwriting input)
#   --manifest  Optional local/remote manifest to package instead of <assets_dir>/manifest.json.
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
export COPYFILE_DISABLE=1

usage() {
    echo "usage: repack-deb.sh [--manifest manifest.json|file://...|http://...|https://...] <input.deb> <bin_dir> <config_root> [assets_dir] [output.deb]" >&2
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

write_manifest_origin() {
    local manifest_source="${1:?write_manifest_origin <manifest_source> <dst>}"
    local dst="${2:?write_manifest_origin <manifest_source> <dst>}"
    python3 - "$manifest_source" "$dst" <<'PY'
import datetime
import json
import pathlib
import sys
import urllib.parse
import urllib.request

raw_source = sys.argv[1]
dst = pathlib.Path(sys.argv[2])
parsed = urllib.parse.urlparse(raw_source)
if parsed.scheme in ("http", "https"):
    source = raw_source
elif parsed.scheme == "file":
    source = str(pathlib.Path(urllib.request.url2pathname(parsed.path)).resolve())
else:
    source = str(pathlib.Path(raw_source).resolve())
dst.write_text(json.dumps({
    "schema": "capsem.manifest_origin.v1",
    "origin": "package",
    "source": source,
    "packaged_at": datetime.datetime.now(datetime.timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z"),
}, sort_keys=True) + "\n")
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
    with urllib.request.urlopen(source, timeout=60) as response:
        dst.write_bytes(response.read())
elif parsed.scheme == "file":
    dst.write_bytes(pathlib.Path(urllib.request.url2pathname(parsed.path)).read_bytes())
elif parsed.scheme:
    raise SystemExit(f"unsupported manifest URL scheme: {parsed.scheme}")
else:
    dst.write_bytes(pathlib.Path(source).read_bytes())
PY
}

materialize_manifest_assets() {
    local assets_view="${1:?materialize_manifest_assets <assets_view> <dst_assets> <local_assets_dir>}"
    local dst_assets="${2:?materialize_manifest_assets <assets_view> <dst_assets> <local_assets_dir>}"
    local local_assets_dir="${3:-}"
    python3 - "$assets_view" "$dst_assets" "$local_assets_dir" <<'PY'
import json
import os
import pathlib
import shutil
import sys

assets_view = pathlib.Path(sys.argv[1])
dst_assets = pathlib.Path(sys.argv[2])
local_assets_dir = sys.argv[3]

machine = os.uname().machine.lower()
arch = "arm64" if machine in ("arm64", "aarch64") else "x86_64"
manifest = json.loads((assets_view / "manifest.json").read_text())
release_id = manifest["assets"]["current"]
arch_assets = manifest["assets"]["releases"][release_id]["arches"].get(arch)
if arch_assets is None:
    print(f"  No {arch} assets in selected manifest; packaged manifest only")
    raise SystemExit(0)

arch_dir = assets_view / arch
if not arch_dir.is_dir():
    if local_assets_dir:
        raise SystemExit(
            f"ERROR: selected manifest references {arch} assets but {arch_dir} is missing"
        )
    print(f"  No local {arch} asset payload; packaged manifest only")
    raise SystemExit(0)

dst_arch = dst_assets / arch
dst_arch.mkdir(parents=True, exist_ok=True)

def hash_filename(logical_name: str, digest: str) -> str:
    prefix = digest[:16]
    if "." in logical_name:
        stem, ext = logical_name.split(".", 1)
        return f"{stem}-{prefix}.{ext}"
    return f"{logical_name}-{prefix}"

for logical_name, meta in sorted(arch_assets.items()):
    hashed_name = hash_filename(logical_name, meta["hash"])
    candidates = [arch_dir / hashed_name, arch_dir / logical_name]
    source = next((path for path in candidates if path.is_file()), None)
    if source is None:
        searched = ", ".join(str(path) for path in candidates)
        raise SystemExit(f"ERROR: missing package asset for {logical_name}; checked {searched}")
    target = dst_arch / hashed_name
    tmp = target.with_suffix(target.suffix + ".tmp")
    shutil.copy2(source, tmp)
    tmp.replace(target)
    print(f"  Added asset: {arch}/{hashed_name}")
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

ASSETS_VIEW="$ASSETS_DIR"
SELECTED_MANIFEST_SOURCE="$ASSETS_DIR/manifest.json"
if [ -n "$MANIFEST_PATH" ]; then
    SELECTED_MANIFEST_SOURCE="$MANIFEST_PATH"
    ASSETS_VIEW="$WORK_DIR/assets-view"
    mkdir -p "$ASSETS_VIEW"
    materialize_manifest_input "$MANIFEST_PATH" "$ASSETS_VIEW/manifest.json"
    if [ -n "$ASSETS_DIR" ]; then
        for arch_dir in "$ASSETS_DIR"/*; do
            [ -d "$arch_dir" ] || continue
            arch_abs="$(cd "$arch_dir" && pwd -P)"
            ln -s "$arch_abs" "$ASSETS_VIEW/$(basename "$arch_dir")"
        done
    fi
fi
if [ -z "$ASSETS_VIEW" ] || [ ! -f "$ASSETS_VIEW/manifest.json" ]; then
    echo "ERROR: manifest not found: $ASSETS_VIEW/manifest.json" >&2
    exit 1
fi
mkdir -p "$WORK_DIR/deb/usr/share/capsem/assets"
cp "$ASSETS_VIEW/manifest.json" "$WORK_DIR/deb/usr/share/capsem/assets/manifest.json"
write_manifest_origin "$SELECTED_MANIFEST_SOURCE" "$WORK_DIR/deb/usr/share/capsem/assets/manifest-origin.json"
materialize_manifest_assets "$ASSETS_VIEW" "$WORK_DIR/deb/usr/share/capsem/assets" "$ASSETS_DIR"

echo "=== Repacking .deb ==="
dpkg-deb -b "$WORK_DIR/deb" "$OUTPUT_DEB"

echo "=== Validating ==="
dpkg-deb --info "$OUTPUT_DEB"

echo "=== Built: $OUTPUT_DEB ==="
ls -lh "$OUTPUT_DEB"
