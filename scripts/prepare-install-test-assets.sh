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
        0,
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
with out.open("wb") as raw:
    with gzip.GzipFile(fileobj=raw, mode="wb", compresslevel=9, mtime=0) as fh:
        fh.write(payload)
PY
}

create_rootfs_scoped_test_obom() {
    local path="${1:?create_rootfs_scoped_test_obom <path>}"
    install -d "$(dirname "$path")"
    python3 - "$path" "$arch" <<'PY'
import json
import sys
from pathlib import Path

out = Path(sys.argv[1])
arch = sys.argv[2]


def is_rootfs_scoped_obom(document: object) -> bool:
    if not isinstance(document, dict) or document.get("bomFormat") != "CycloneDX":
        return False
    metadata = document.get("metadata")
    component = metadata.get("component") if isinstance(metadata, dict) else None
    properties = component.get("properties") if isinstance(component, dict) else None
    if not isinstance(properties, list) or not any(
        isinstance(prop, dict)
        and prop.get("name") == "capsem:evidence:scope"
        and prop.get("value") == "exported-rootfs"
        for prop in properties
    ):
        return False
    components = document.get("components")
    if not isinstance(components, list) or not any(
        isinstance(item, dict)
        and isinstance(item.get("purl"), str)
        and item["purl"].startswith("pkg:deb/debian/")
        for item in components
    ):
        return False

    def contains_live_host_marker(value: object) -> bool:
        if isinstance(value, dict):
            if value.get("name") == "cdx:osquery:category":
                return True
            return any(contains_live_host_marker(item) for item in value.values())
        if isinstance(value, list):
            return any(contains_live_host_marker(item) for item in value)
        return False

    return not contains_live_host_marker(document)


if out.is_file():
    try:
        existing = json.loads(out.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError):
        existing = None
    if is_rootfs_scoped_obom(existing):
        raise SystemExit(0)

document = {
    "bomFormat": "CycloneDX",
    "specVersion": "1.6",
    "metadata": {
        "component": {
            "name": f"capsem-install-test-rootfs-{arch}",
            "type": "operating-system",
            "version": "install-test",
            "properties": [
                {"name": "capsem:evidence:scope", "value": "exported-rootfs"},
                {"name": "capsem:guest:architecture", "value": arch},
            ],
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
    "components": [
        {
            "type": "library",
            "name": "base-files",
            "version": "install-test",
            "purl": f"pkg:deb/debian/base-files@install-test?arch={arch}",
        }
    ],
}
out.write_text(json.dumps(document, indent=2) + "\n", encoding="utf-8")
PY
}

create_minimal_software_inventory_if_missing() {
    local path="${1:?create_minimal_software_inventory_if_missing <path>}"
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
    "schema": "capsem.profile_software_inventory.v1",
    "architecture": arch,
    "packages": [
        {
            "name": "capsem-install-test-tool",
            "version": "1.0.0",
            "source": "fixture",
            "architecture": arch,
        }
    ],
}
out.write_text(json.dumps(document, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY
}

write_if_missing "$ASSETS_DIR/$arch/vmlinuz" "capsem install-test kernel $arch"
create_minimal_initrd_if_missing "$ASSETS_DIR/$arch/initrd.img"
write_if_missing "$ASSETS_DIR/$arch/rootfs.erofs" "capsem install-test rootfs $arch"
create_rootfs_scoped_test_obom "$ASSETS_DIR/$arch/obom.cdx.json"
create_minimal_software_inventory_if_missing "$ASSETS_DIR/$arch/software-inventory.json"

rm -rf "$ASSETS_DIR/current"
install -d "$ASSETS_DIR/current"
cp -R "$ASSETS_DIR/$arch/." "$ASSETS_DIR/current/"

VERSION=$(grep '^version' "$ROOT/Cargo.toml" | head -1 | sed 's/.*"\(.*\)"/\1/')
cd "$ROOT"
cargo run -p capsem-admin -- manifest generate "$ASSETS_DIR" --version "$VERSION"
