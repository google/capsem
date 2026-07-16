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
#   --manifest  Optional manifest URL to record for postinstall hydration.
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
#   /usr/share/capsem/assets/manifest-metadata.json
#   /usr/share/capsem/profiles/
#   DEBIAN/preinst script
#   DEBIAN/postinst script
set -euo pipefail
export COPYFILE_DISABLE=1

embed_install_diagnostics() {
    local maintainer_script="$1"
    local combined="${maintainer_script}.with-install-diagnostics"
    {
        head -n 1 "$maintainer_script"
        sed -n '2,$p' "$SCRIPT_DIR/pkg-scripts/install-diagnostics"
        sed -n '2,$p' "$maintainer_script"
    } > "$combined"
    mv "$combined" "$maintainer_script"
}

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

write_manifest_metadata() {
    local manifest_source="${1:?write_manifest_metadata <manifest_source> <package_version> <dst>}"
    local package_version="${2:?write_manifest_metadata <manifest_source> <package_version> <dst>}"
    local dst="${3:?write_manifest_metadata <manifest_source> <package_version> <dst>}"
    python3 - "$manifest_source" "$package_version" "$dst" <<'PY'
import datetime
import json
import pathlib
import sys
import urllib.parse

raw_source = sys.argv[1]
package_version = sys.argv[2]
dst = pathlib.Path(sys.argv[3])
parsed = urllib.parse.urlparse(raw_source)
if parsed.scheme not in ("http", "https", "file"):
    raise SystemExit(f"manifest must be a URL: {raw_source}")
parts = [part for part in parsed.path.split("/") if part]
public_channel = (
    parts[-2]
    if len(parts) >= 3
    and parts[-3] == "assets"
    and parts[-2] in ("stable", "nightly")
    and parts[-1] == "manifest.json"
    else None
)
fetched_at = datetime.datetime.now(datetime.timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")
dst.write_text(json.dumps({
    "schema": "capsem.manifest_metadata.v1",
    "origin": "package",
    "manifest_url": raw_source,
    "channel": public_channel or "corp",
    "channel_kind": "public" if public_channel else "corporate",
    "channel_locked": public_channel is None,
    "fetched_at": fetched_at,
    "packaged_at": fetched_at,
    "package_version": package_version,
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

is_elf_binary() {
    local path="${1:?is_elf_binary <path>}"
    local magic
    magic="$(LC_ALL=C head -c 4 "$path" 2>/dev/null || true)"
    [ "$magic" = $'\177ELF' ]
}

strip_packaged_binaries() {
    local stripped=0
    local path
    while IFS= read -r -d '' path; do
        if ! is_elf_binary "$path"; then
            continue
        fi
        local description candidate strip_tool
        description="$(LC_ALL=C file -b "$path")"
        case "$description" in
            *x86-64*) candidate="x86_64-linux-gnu-strip" ;;
            *aarch64*|*ARM64*) candidate="aarch64-linux-gnu-strip" ;;
            *) candidate="strip" ;;
        esac
        if command -v "$candidate" >/dev/null 2>&1; then
            strip_tool="$candidate"
        elif command -v strip >/dev/null 2>&1; then
            strip_tool="strip"
        else
            echo "ERROR: no strip tool available for $description ($path)" >&2
            return 1
        fi
        if ! "$strip_tool" --strip-unneeded "$path"; then
            echo "ERROR: $strip_tool failed for $description ($path)" >&2
            return 1
        fi
        stripped=$((stripped + 1))
    done < <(find "$WORK_DIR/deb/usr/bin" -maxdepth 1 -type f -print0)

    if [ "$stripped" -gt 0 ]; then
        echo "  Stripped ELF binaries: $stripped"
    fi
}

ensure_deb_dependency() {
    local control="${1:?ensure_deb_dependency <control> <dependency>}"
    local dependency="${2:?ensure_deb_dependency <control> <dependency>}"
    python3 - "$control" "$dependency" <<'PY'
import pathlib
import re
import sys

control = pathlib.Path(sys.argv[1])
dependency = sys.argv[2]
text = control.read_text()
match = re.search(r"(?ms)^Depends:\s*(.*?)(?=^[^ \t]|\Z)", text)
if match is None:
    insert_at = len(text)
    if not text.endswith("\n"):
        text += "\n"
        insert_at += 1
    text = text[:insert_at] + f"Depends: {dependency}\n" + text[insert_at:]
else:
    raw = match.group(1)
    deps = [item.strip() for item in raw.replace("\n", " ").split(",") if item.strip()]
    names = {re.split(r"\s*[ (]", item, maxsplit=1)[0] for item in deps}
    if dependency not in names:
        deps.append(dependency)
        replacement = "Depends: " + ", ".join(deps) + "\n"
        text = text[: match.start()] + replacement + text[match.end():]
control.write_text(text)
PY
}

echo "=== Extracting .deb ==="
dpkg-deb -R "$INPUT_DEB" "$WORK_DIR/deb"

echo "=== Ensuring Debian package dependencies ==="
ensure_deb_dependency "$WORK_DIR/deb/DEBIAN/control" "libxdo3"

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
strip_packaged_binaries

echo "=== Adding maintainer scripts ==="
cp "$SCRIPT_DIR/deb-preinst.sh" "$WORK_DIR/deb/DEBIAN/preinst"
embed_install_diagnostics "$WORK_DIR/deb/DEBIAN/preinst"
chmod 755 "$WORK_DIR/deb/DEBIAN/preinst"
cp "$SCRIPT_DIR/deb-postinst.sh" "$WORK_DIR/deb/DEBIAN/postinst"
embed_install_diagnostics "$WORK_DIR/deb/DEBIAN/postinst"
chmod 755 "$WORK_DIR/deb/DEBIAN/postinst"

if [ ! -d "$CONFIG_ROOT/profiles" ]; then
    echo "ERROR: materialized profiles not found: $CONFIG_ROOT/profiles" >&2
    echo "Run: just _materialize-config" >&2
    exit 1
fi
echo "=== Adding materialized profiles ==="
mkdir -p "$WORK_DIR/deb/usr/share/capsem/profiles"
cp -R "$CONFIG_ROOT/profiles/." "$WORK_DIR/deb/usr/share/capsem/profiles/"

PACKAGE_VERSION="$(dpkg-deb -f "$INPUT_DEB" Version)"
if [ -n "$MANIFEST_PATH" ]; then
    SELECTED_MANIFEST_SOURCE="$MANIFEST_PATH"
else
    if [ -z "$ASSETS_DIR" ] || [ ! -f "$ASSETS_DIR/manifest.json" ]; then
        echo "ERROR: manifest not found: $ASSETS_DIR/manifest.json" >&2
        exit 1
    fi
    SELECTED_MANIFEST_SOURCE="$(file_url "$ASSETS_DIR/manifest.json")"
fi
mkdir -p "$WORK_DIR/deb/usr/share/capsem/assets"
write_manifest_metadata "$SELECTED_MANIFEST_SOURCE" "$PACKAGE_VERSION" "$WORK_DIR/deb/usr/share/capsem/assets/manifest-metadata.json"

echo "=== Repacking .deb ==="
dpkg-deb -b "$WORK_DIR/deb" "$OUTPUT_DEB"

echo "=== Validating ==="
dpkg-deb --info "$OUTPUT_DEB"

echo "=== Built: $OUTPUT_DEB ==="
ls -lh "$OUTPUT_DEB"
