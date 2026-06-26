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

if [[ -L "$DST" ]]; then
    echo "Removing asset symlink: $DST -> $(readlink "$DST")"
    rm "$DST"
fi

if [[ -e "$DST" && "$SRC" -ef "$DST" ]]; then
    echo "ERROR: source and destination assets directories must be distinct: $SRC" >&2
    exit 1
fi

mkdir -p "$DST/$ARCH"

cp "$SRC/manifest.json" "$DST/manifest.json.tmp"
mv "$DST/manifest.json.tmp" "$DST/manifest.json"
python3 - "$SRC/manifest.json" "$DST/manifest-origin.json" <<'PY'
import json
import pathlib
import sys

manifest = pathlib.Path(sys.argv[1]).resolve()
dst = pathlib.Path(sys.argv[2])
tmp = dst.with_suffix(dst.suffix + ".tmp")
tmp.write_text(
    json.dumps(
        {
            "schema": "capsem.manifest_origin.v1",
            "origin": "local-dev-sync",
            "source": str(manifest),
        },
        sort_keys=True,
    )
    + "\n",
    encoding="utf-8",
)
tmp.replace(dst)
PY

# Materialize the installed layout from the manifest. Local build output may
# be literal (`rootfs.erofs`) while downloaded/reconciled output is
# hash-prefixed (`rootfs-<hash16>.erofs`); the installed tree is always
# hash-prefixed so ManifestV2::resolve and profile boot pins use one shape.
python3 - "$SRC" "$DST" "$ARCH" <<'PY'
import json
import shutil
import sys
from pathlib import Path

src = Path(sys.argv[1])
dst = Path(sys.argv[2])
arch = sys.argv[3]

manifest = json.loads((src / "manifest.json").read_text())
asset_version = manifest["assets"]["current"]
assets = manifest["assets"]["releases"][asset_version]["arches"][arch]

def hash_filename(logical_name: str, digest: str) -> str:
    prefix = digest[:16]
    if "." in logical_name:
        stem, ext = logical_name.split(".", 1)
        return f"{stem}-{prefix}.{ext}"
    return f"{logical_name}-{prefix}"

for logical_name, meta in sorted(assets.items()):
    hashed_name = hash_filename(logical_name, meta["hash"])
    candidates = [src / arch / hashed_name, src / arch / logical_name]
    source = next((p for p in candidates if p.is_file()), None)
    if source is None:
        searched = ", ".join(str(p) for p in candidates)
        raise SystemExit(f"ERROR: missing source asset for {logical_name}; checked {searched}")
    target = dst / arch / hashed_name
    if target.exists() and source.samefile(target):
        continue
    tmp = target.with_suffix(target.suffix + ".tmp")
    shutil.copy2(source, tmp)
    tmp.replace(target)

expected = {hash_filename(name, meta["hash"]) for name, meta in assets.items()}
for candidate in (dst / arch).iterdir():
    if not candidate.is_file():
        continue
    name = candidate.name
    if "-" not in name or name in expected:
        continue
    stem = name.split("-", 1)[0]
    if stem not in {logical.split(".", 1)[0] for logical in assets}:
        continue
    candidate.unlink()
PY

# Drop legacy v1 layout directories that ManifestV2::resolve() no longer reads.
# They would otherwise keep occupying ~450MB/install.
for legacy in "$DST"/v1.0.*; do
    [[ -d "$legacy" ]] || continue
    rm -rf "$legacy"
    echo "  removed stale $(basename "$legacy")"
done

# Surface any hash drift between the manifest and the file on disk.
if command -v b3sum >/dev/null 2>&1; then
    ROOTFS=$(python3 -c "import json,sys;m=json.load(open('$SRC/manifest.json'));v=m['assets']['current'];a=m['assets']['releases'][v]['arches']['$ARCH'];print('rootfs.erofs' if 'rootfs.erofs' in a else '')" 2>/dev/null || true)
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
