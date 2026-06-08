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

# Short-circuit when ~/.capsem/assets is a symlink back to this repo's assets/.
# Remove a stale symlink to another worktree before copying; otherwise mkdir/cp
# silently populate the wrong tree and the installed service reports missing
# hash-named assets.
if [[ -e "$DST" && "$SRC" -ef "$DST" ]]; then
    echo "Skipped sync: $DST resolves to $SRC (symlinked dev layout)"
    exit 0
fi

if [[ -L "$DST" ]]; then
    echo "Removing stale asset symlink: $DST -> $(readlink "$DST")"
    rm "$DST"
fi

mkdir -p "$DST/$ARCH"

cp "$SRC/manifest.json" "$DST/manifest.json.tmp"
mv "$DST/manifest.json.tmp" "$DST/manifest.json"

# Per-file copy so one "identical" pair doesn't kill the loop. Same-inode
# pairs happen when individual files are hardlinked (APFS clonefile from a
# prior `just install` run) or when the src/dst arch dir is symlinked.
for src_file in "$SRC/$ARCH"/*; do
    [[ -f "$src_file" ]] || continue
    dst_file="$DST/$ARCH/$(basename "$src_file")"
    if [[ "$src_file" -ef "$dst_file" ]]; then
        continue
    fi
    cp -f "$src_file" "$dst_file"
done

# Drop legacy v1 layout directories that ManifestV2::resolve() no longer reads.
# They would otherwise keep occupying ~450MB/install.
for legacy in "$DST"/v1.0.*; do
    [[ -d "$legacy" ]] || continue
    rm -rf "$legacy"
    echo "  removed stale $(basename "$legacy")"
done

# Surface any hash drift between the manifest and the file on disk.
if command -v b3sum >/dev/null 2>&1; then
    ROOTFS=$(python3 -c "import json,sys;m=json.load(open('$SRC/manifest.json'));v=m['assets']['current'];a=m['assets']['releases'][v]['arches']['$ARCH'];print('rootfs.erofs' if 'rootfs.erofs' in a else 'rootfs.squashfs')" 2>/dev/null || true)
    EXPECTED=$(python3 -c "import json,sys;m=json.load(open('$SRC/manifest.json'));v=m['assets']['current'];a=m['assets']['releases'][v]['arches']['$ARCH'];r='$ROOTFS';print(a[r]['hash'])" 2>/dev/null || true)
    HASHED=""
    if [[ -n "$ROOTFS" && -n "$EXPECTED" ]]; then
        prefix="${EXPECTED:0:16}"
        stem="${ROOTFS%.*}"
        ext="${ROOTFS#*.}"
        HASHED="$stem-$prefix.$ext"
    fi
    CHECK_PATH="$DST/$ARCH/$HASHED"
    if [[ ! -f "$CHECK_PATH" ]]; then
        CHECK_PATH="$DST/$ARCH/$ROOTFS"
    fi
    ACTUAL=""
    if [[ -f "$CHECK_PATH" ]]; then
        ACTUAL=$(b3sum --no-names "$CHECK_PATH" 2>/dev/null | awk '{print $1}')
    fi
    if [[ -n "$EXPECTED" && -n "$ACTUAL" && "$EXPECTED" != "$ACTUAL" ]]; then
        echo "WARNING: $ROOTFS hash does not match manifest"
        echo "  expected: $EXPECTED"
        echo "  actual:   $ACTUAL"
    fi
fi

echo "Synced dev assets -> $DST ($ARCH)"
