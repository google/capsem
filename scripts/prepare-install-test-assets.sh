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

create_minimal_initrd_if_missing() {
    local path="${1:?create_minimal_initrd_if_missing <path>}"
    if [ -f "$path" ] && gzip -t "$path" >/dev/null 2>&1; then
        return
    fi

    install -d "$(dirname "$path")"
    python3 - "$path" "$arch" <<'PY'
import gzip
import sys
import time
from pathlib import Path


def _pad4(length: int) -> bytes:
    return b"\0" * ((4 - (length % 4)) % 4)


def _newc_record(name: str, data: bytes, mode: int, ino: int) -> bytes:
    name_bytes = name.encode("utf-8") + b"\0"
    fields = [
        ino,
        mode,
        0,
        0,
        1,
        int(time.time()),
        len(data),
        0,
        0,
        0,
        0,
        len(name_bytes),
        0,
    ]
    header = b"070701" + "".join(f"{field:08x}" for field in fields).encode("ascii")
    return header + name_bytes + _pad4(len(header) + len(name_bytes)) + data + _pad4(len(data))


out = Path(sys.argv[1])
arch = sys.argv[2]
payload = _newc_record(
    "README",
    f"capsem install-test initrd {arch}\n".encode("utf-8"),
    0o100644,
    1,
)
payload += _newc_record("TRAILER!!!", b"", 0, 2)
with gzip.open(out, "wb", compresslevel=9) as fh:
    fh.write(payload)
PY
}

create_minimal_obom_if_missing() {
    local path="${1:?create_minimal_obom_if_missing <path>}"
    if [ -f "$path" ]; then
        return
    fi

    install -d "$(dirname "$path")"
    python3 - "$path" "$arch" <<'PY'
import json
import sys
from pathlib import Path

out = Path(sys.argv[1])
arch = sys.argv[2]
document = {
    "bomFormat": "CycloneDX",
    "specVersion": "1.6",
    "metadata": {
        "component": {
            "name": f"capsem-install-test-rootfs-{arch}",
            "type": "operating-system",
        },
        "tools": {
            "components": [
                {
                    "name": "capsem-install-test-assets",
                    "type": "application",
                    "version": "1",
                }
            ]
        },
    },
    "components": [],
}
out.write_text(json.dumps(document, indent=2) + "\n", encoding="utf-8")
PY
}

write_if_missing "$ASSETS_DIR/$arch/vmlinuz" "capsem install-test kernel $arch"
create_minimal_initrd_if_missing "$ASSETS_DIR/$arch/initrd.img"
write_if_missing "$ASSETS_DIR/$arch/rootfs.erofs" "capsem install-test rootfs $arch"
create_minimal_obom_if_missing "$ASSETS_DIR/$arch/obom.cdx.json"

rm -rf "$ASSETS_DIR/current"
install -d "$ASSETS_DIR/current"
cp -R "$ASSETS_DIR/$arch/." "$ASSETS_DIR/current/"

VERSION=$(grep '^version' "$ROOT/Cargo.toml" | head -1 | sed 's/.*"\(.*\)"/\1/')
cd "$ROOT"
cargo run -p capsem-admin -- manifest generate "$ASSETS_DIR" --version "$VERSION"
