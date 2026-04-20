#!/bin/bash
# sync-dev-assets.sh -- Mirror a locally built assets/ tree into ~/.capsem/assets/.
#
# Usage: sync-dev-assets.sh <src_assets_dir> <dst_assets_dir>
#
# `just install` ships a .pkg/.deb with only manifest.json (binaries and assets
# are on independent shipping cadences -- see sprints/orthogonal-ci). For the
# local dev install we short-circuit the network download by copying the
# authoritative, freshly-built local files into the installed tree. The
# service's ManifestV2::resolve() reads $dst/$arch/{hash_filename}, which is
# exactly the layout _pack-initrd produces.
set -euo pipefail

SRC="${1:?usage: sync-dev-assets.sh <src_assets_dir> <dst_assets_dir>}"
DST="${2:?usage: sync-dev-assets.sh <src_assets_dir> <dst_assets_dir>}"

ARCH=$(uname -m)
[[ "$ARCH" == "aarch64" ]] && ARCH="arm64"

if [[ ! -f "$SRC/manifest.json" ]]; then
    echo "ERROR: $SRC/manifest.json not found -- run 'just build-assets' first" >&2
    exit 1
fi
if [[ ! -d "$SRC/$ARCH" ]]; then
    echo "ERROR: $SRC/$ARCH not found -- run 'just build-assets $ARCH' first" >&2
    exit 1
fi

mkdir -p "$DST/$ARCH"

cp "$SRC/manifest.json" "$DST/manifest.json.tmp"
mv "$DST/manifest.json.tmp" "$DST/manifest.json"

cp -f "$SRC/$ARCH"/* "$DST/$ARCH/"

# Drop legacy v1 layout directories that ManifestV2::resolve() no longer reads.
# They would otherwise keep occupying ~450MB/install.
for legacy in "$DST"/v1.0.*; do
    [[ -d "$legacy" ]] || continue
    rm -rf "$legacy"
    echo "  removed stale $(basename "$legacy")"
done

# Surface any hash drift between the manifest and the file on disk.
if command -v b3sum >/dev/null 2>&1; then
    EXPECTED=$(python3 -c "import json,sys;m=json.load(open('$SRC/manifest.json'));v=m['assets']['current'];print(m['assets']['releases'][v]['arches']['$ARCH']['rootfs.squashfs']['hash'])" 2>/dev/null || true)
    ACTUAL=$(b3sum --no-names "$DST/$ARCH/rootfs.squashfs" 2>/dev/null | awk '{print $1}')
    if [[ -n "$EXPECTED" && -n "$ACTUAL" && "$EXPECTED" != "$ACTUAL" ]]; then
        echo "WARNING: rootfs.squashfs hash does not match manifest"
        echo "  expected: $EXPECTED"
        echo "  actual:   $ACTUAL"
    fi
fi

echo "Synced dev assets -> $DST ($ARCH)"
