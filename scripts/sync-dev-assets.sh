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

# Dev-key signing for the locally built manifest. Release binaries refuse
# to boot when manifest.json has no sibling manifest.json.minisig (see
# crates/capsem-core/src/asset_manager.rs::load_verified_manifest_for_assets).
# `just install` ships release binaries, so without a dev-side signature
# every locally built sandbox fails to boot with "manifest signature
# missing". The binary additionally trusts a sibling manifest-sign.dev.pub
# via verify_manifest_with_baked_or_dev_key; this block generates that
# key on first install, signs the local manifest, and deploys the pubkey
# next to the manifest so dev boots succeed.
sign_manifest_with_dev_key() {
    local manifest="$1"
    local dst_dir="$2"
    if ! command -v minisign >/dev/null 2>&1; then
        echo "WARNING: minisign not installed; locally built manifest will be"
        echo "         unsigned and release binaries will refuse to boot it."
        echo "         Fix: brew install minisign (macOS) or apt install minisign (Linux)."
        return 0
    fi
    local key_dir="$HOME/.capsem/dev-keys"
    local priv="$key_dir/manifest-sign.dev.key"
    local pub="$key_dir/manifest-sign.dev.pub"
    mkdir -p "$key_dir"
    chmod 700 "$key_dir"
    if [[ ! -f "$priv" || ! -f "$pub" ]]; then
        echo "Generating dev minisign keypair at $key_dir (first install)"
        # -W: no password, so sync-dev-assets can run unattended.
        minisign -G -f -W -p "$pub" -s "$priv" >/dev/null
        chmod 600 "$priv"
    fi
    # Sign the manifest as a sibling .minisig. -f overwrites a stale sig
    # from a previous run. No comment so stdin prompting is skipped.
    minisign -S -f -s "$priv" -m "$manifest" -t "capsem dev key" >/dev/null
    # Deploy pubkey next to manifest -- capsem-core reads it from there.
    cp -f "$pub" "$dst_dir/manifest-sign.dev.pub"
}

# Short-circuit when ~/.capsem/assets is a symlink back to the repo's
# assets/ (the dev-loop convenience set up by `just install` for the
# hot-iteration flow). cp would otherwise exit 1 on every "identical
# (not copied)" pair and kill the recipe under `set -e`.
if [[ "$SRC" -ef "$DST" ]]; then
    echo "Skipped sync: $DST resolves to $SRC (symlinked dev layout)"
    # Still sign the (shared) manifest in-place -- the release binary
    # reads it from $DST, which here points at $SRC, so signing either
    # lands the .minisig where the binary looks.
    sign_manifest_with_dev_key "$DST/manifest.json" "$DST"
    exit 0
fi

cp "$SRC/manifest.json" "$DST/manifest.json.tmp"
mv "$DST/manifest.json.tmp" "$DST/manifest.json"

# Per-file copy so one "identical" pair doesn't kill the loop. Same-inode
# pairs happen when individual files are hardlinked (APFS clonefile from a
# prior `just install` run) or when the src/dst arch dir is symlinked.
for src_file in "$SRC/$ARCH"/*; do
    [[ -e "$src_file" ]] || continue
    dst_file="$DST/$ARCH/$(basename "$src_file")"
    if [[ "$src_file" -ef "$dst_file" ]]; then
        continue
    fi
    cp -f "$src_file" "$dst_file"
done

# Sign the freshly copied manifest and deploy the dev pubkey next to it.
sign_manifest_with_dev_key "$DST/manifest.json" "$DST"

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
