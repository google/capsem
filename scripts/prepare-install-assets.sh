#!/bin/bash
# prepare-install-assets.sh -- Build and materialize signed assets for install E2E.
#
# Usage:
#   scripts/prepare-install-assets.sh [assets_dir] [cargo_toml] [arch]
#
# Defaults:
#   assets_dir = assets
#   cargo_toml = Cargo.toml
#   arch       = $INSTALL_ARCH or host uname -m
set -euo pipefail

normalize_arch() {
    case "$1" in
        arm64|aarch64) echo "arm64" ;;
        x86_64|amd64) echo "x86_64" ;;
        *)
            echo "ERROR: unsupported install arch '$1' (expected arm64 or x86_64)" >&2
            exit 1
            ;;
    esac
}

ASSETS_DIR="${1:-assets}"
CARGO_TOML="${2:-Cargo.toml}"
ARCH_INPUT="${3:-${INSTALL_ARCH:-$(uname -m)}}"
INSTALL_ARCH="$(normalize_arch "$ARCH_INPUT")"

echo "=== Preparing install assets for $INSTALL_ARCH ==="
for f in vmlinuz initrd.img rootfs.squashfs; do
    test -f "$ASSETS_DIR/$INSTALL_ARCH/$f" || {
        echo "ERROR: missing asset: $ASSETS_DIR/$INSTALL_ARCH/$f" >&2
        echo "       Build assets on the host first: just build-assets $INSTALL_ARCH" >&2
        exit 1
    }
done

echo "=== Regenerating manifest + hash aliases ==="
(
    cd "$ASSETS_DIR"
    b3sum "$INSTALL_ARCH/vmlinuz" "$INSTALL_ARCH/initrd.img" "$INSTALL_ARCH/rootfs.squashfs" > B3SUMS
)
python3 scripts/gen_manifest.py "$ASSETS_DIR" "$CARGO_TOML"
python3 scripts/create_hash_assets.py "$ASSETS_DIR"
bash scripts/sync-dev-assets.sh "$ASSETS_DIR" "$ASSETS_DIR"
bash scripts/verify-local-manifest-signature.sh "$ASSETS_DIR" config/manifest-sign.pub

echo "Install asset prep complete: $ASSETS_DIR ($INSTALL_ARCH)"
