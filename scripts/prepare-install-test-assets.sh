#!/usr/bin/env bash
set -euo pipefail

ROOT="${CAPSEM_REPO_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
ASSETS_DIR="${CAPSEM_ASSETS_DIR:-$ROOT/assets}"

arch="${CAPSEM_ARCH:-$(uname -m)}"
case "$arch" in
    arm64|aarch64)
        arch="arm64"
        ;;
    x86_64|amd64)
        arch="x86_64"
        ;;
    *)
        echo "ERROR: unsupported install-test asset arch: $arch" >&2
        exit 1
        ;;
esac

write_if_missing() {
    local path="${1:?write_if_missing <path> <content>}"
    local content="${2:?write_if_missing <path> <content>}"
    if [ ! -f "$path" ]; then
        install -d "$(dirname "$path")"
        printf '%s\n' "$content" > "$path"
    fi
}

write_if_missing "$ASSETS_DIR/$arch/vmlinuz" "capsem install-test kernel $arch"
write_if_missing "$ASSETS_DIR/$arch/initrd.img" "capsem install-test initrd $arch"
write_if_missing "$ASSETS_DIR/$arch/rootfs.erofs" "capsem install-test rootfs $arch"

rm -rf "$ASSETS_DIR/current"
install -d "$ASSETS_DIR/current"
cp -R "$ASSETS_DIR/$arch/." "$ASSETS_DIR/current/"

VERSION=$(grep '^version' "$ROOT/Cargo.toml" | head -1 | sed 's/.*"\(.*\)"/\1/')
cd "$ROOT"
cargo run -p capsem-admin -- manifest generate "$ASSETS_DIR" --version "$VERSION"
