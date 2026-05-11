#!/usr/bin/env bash
# Validate that a built rootfs contains the release-critical guest artifacts.
set -euo pipefail

if [ "$#" -ne 1 ]; then
    echo "usage: $0 <rootfs.squashfs>" >&2
    exit 2
fi

ROOTFS="$1"
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [ ! -f "$ROOTFS" ]; then
    echo "::error::rootfs.squashfs not found at $ROOTFS" >&2
    exit 1
fi

if command -v uv >/dev/null 2>&1; then
    PYTHON=(uv run python3)
else
    PYTHON=(python3)
fi

REQUIRED=$(
    cd "$ROOT_DIR"
    PYTHONPATH="$ROOT_DIR/src${PYTHONPATH:+:$PYTHONPATH}" "${PYTHON[@]}" - <<'PY'
from capsem.builder.docker import (
    GUEST_BINARIES,
    ROOTFS_SCRIPTS,
    ROOTFS_SCRIPT_DIRS,
    ROOTFS_SUPPORT_FILES,
)

for name in [*GUEST_BINARIES, *ROOTFS_SCRIPTS]:
    print(f"file /usr/local/bin/{name}")
for name in ROOTFS_SCRIPT_DIRS:
    target = "capsem-tests" if name == "diagnostics" else name
    print(f"dir /usr/local/lib/{target}")
for name in ROOTFS_SUPPORT_FILES:
    target = {
        "capsem-bashrc": "/etc/capsem-bashrc",
        "banner.txt": "/etc/capsem-banner.txt",
        "tips.txt": "/etc/capsem-tips.txt",
    }[name]
    print(f"file {target}")
print("symlink /usr/local/bin/capsem-test")
PY
)

MOUNT=$(mktemp -d)
cleanup() {
    sudo umount "$MOUNT" >/dev/null 2>&1 || true
    rmdir "$MOUNT" >/dev/null 2>&1 || true
}
trap cleanup EXIT

sudo mount -t squashfs -o loop,ro "$ROOTFS" "$MOUNT"

MISSING=()
while read -r kind path; do
    [ -n "${kind:-}" ] || continue
    case "$kind" in
        file)
            [ -f "$MOUNT$path" ] || MISSING+=("$path")
            ;;
        dir)
            [ -d "$MOUNT$path" ] || MISSING+=("$path/")
            ;;
        symlink)
            [ -L "$MOUNT$path" ] || MISSING+=("$path -> symlink")
            ;;
        *)
            echo "unknown rootfs requirement kind: $kind" >&2
            exit 2
            ;;
    esac
done <<< "$REQUIRED"

if [ "${#MISSING[@]}" -ne 0 ]; then
    printf '::error::rootfs is missing required artifact(s):' >&2
    printf ' %s' "${MISSING[@]}" >&2
    printf '\n' >&2
    exit 1
fi

echo "All required rootfs artifacts present in $ROOTFS"
