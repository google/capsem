#!/bin/bash
# Boot a real Capsem guest on the physical Mac using the exact .pkg payload.
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
PKG="${1:?usage: prove-macos-package-boot.sh PACKAGE VERSION}"
VERSION="${2:?missing package version}"
WORK_ROOT="$ROOT/target/macos-package-boot"
EXPANDED="$WORK_ROOT/expanded"
CAPSEM_HOME_DIR="$WORK_ROOT/home"
RUN_DIR=$(mktemp -d /tmp/capsem-pkg-boot.XXXXXX)

[ "$(uname -s)" = "Darwin" ] || {
    echo "ERROR: macOS package guest-boot proof requires macOS" >&2
    exit 1
}
[ -s "$PKG" ] || {
    echo "ERROR: package is missing or empty: $PKG" >&2
    exit 1
}

stop_isolated_processes() {
    local name
    for name in \
        capsem-service capsem-tray capsem-gateway capsem-process \
        capsem-mcp-aggregator capsem-mcp-builtin
    do
        pkill -9 -f "$CAPSEM_HOME_DIR/bin/$name" 2>/dev/null || true
    done
}
cleanup() {
    stop_isolated_processes
    rm -rf "$RUN_DIR"
}
trap cleanup EXIT

rm -rf "$WORK_ROOT"
mkdir -p "$WORK_ROOT"
/usr/sbin/pkgutil --expand-full "$PKG" "$EXPANDED"

SHARE_COUNT=$(find "$EXPANDED" -type d -path '*/usr/local/share/capsem' | wc -l | tr -d ' ')
[ "$SHARE_COUNT" -eq 1 ] || {
    echo "ERROR: expected one Capsem package share, found $SHARE_COUNT" >&2
    exit 1
}
PKG_SHARE=$(find "$EXPANDED" -type d -path '*/usr/local/share/capsem')

echo "=== Materializing exact package payload for physical-host VZ proof ==="
CAPSEM_HOME="$CAPSEM_HOME_DIR" \
CAPSEM_RUN_DIR="$RUN_DIR" \
    bash "$ROOT/scripts/simulate-install.sh" \
        "$PKG_SHARE/bin" \
        "$ROOT/assets" \
        "$PKG_SHARE"

for binary in "$PKG_SHARE"/bin/capsem*; do
    name=$(basename "$binary")
    "$CAPSEM_HOME_DIR/bin/$name" --version | grep -F "$VERSION"
done

echo "=== Booting real Capsem guest from exact package binaries and profiles ==="
CAPSEM_HOME="$CAPSEM_HOME_DIR" \
CAPSEM_RUN_DIR="$RUN_DIR" \
CAPSEM_ASSETS_DIR="$CAPSEM_HOME_DIR/assets" \
CAPSEM_PROFILES_DIR="$CAPSEM_HOME_DIR/profiles" \
    python3 "$ROOT/scripts/prove-installed-shell.py" \
        --capsem "$CAPSEM_HOME_DIR/bin/capsem" \
        --marker CAPSEM_MACOS_PACKAGE_VM_BOOT_OK \
        --session-name macos-package-vm-boot \
        --profile code \
        --timeout 300

python3 - "$WORK_ROOT/report.json" "$PKG" "$VERSION" <<'PY'
import hashlib
import json
from pathlib import Path
import sys

package = Path(sys.argv[2]).resolve()
report = {
    "schema": "capsem.macos_package_boot.v1",
    "package": str(package),
    "package_sha256": hashlib.sha256(package.read_bytes()).hexdigest(),
    "package_version": sys.argv[3],
    "package_payload_materialized": True,
    "session_created": True,
    "guest_vm_booted": True,
    "guest_shell_marker": "CAPSEM_MACOS_PACKAGE_VM_BOOT_OK",
}
Path(sys.argv[1]).write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
PY

echo "Exact macOS package guest-VM boot proof passed: $WORK_ROOT/report.json"
