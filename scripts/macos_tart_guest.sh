#!/bin/bash
# Install and exercise the shared Capsem package inside a clean Tart macOS VM.
set -euo pipefail

VERSION="${1:?usage: macos_tart_guest.sh VERSION MANIFEST_URL CHANNEL}"
MANIFEST_URL="${2:?missing manifest URL}"
CHANNEL="${3:?missing channel}"
SHARE="/Volumes/My Shared Files/capsem-release"
PKG="/Volumes/My Shared Files/capsem-release/Capsem.pkg"
CAPSEM_HOME="$HOME/.capsem"
CAPSEM_BIN_DIR="$CAPSEM_HOME/bin"
CAPSEM="$CAPSEM_BIN_DIR/capsem"
VERIFY="$SHARE/verify-installed-release.py"
INSTALL_USER_REQUEST="$SHARE/macos-install-user-request.sh"
REPORT="$SHARE/report.json"
BINARIES=(
    capsem
    capsem-service
    capsem-process
    capsem-tui
    capsem-mcp
    capsem-mcp-aggregator
    capsem-mcp-builtin
    capsem-gateway
    capsem-tray
    capsem-admin
)

exec > >(tee "$SHARE/guest.log") 2>&1
export PATH="$CAPSEM_BIN_DIR:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"

case "$CHANNEL" in
    stable|nightly) ;;
    *) echo "ERROR: channel must be stable or nightly (got: $CHANNEL)" >&2; exit 2 ;;
esac

test -s "$PKG"
test -f "$VERIFY"
test -f "$INSTALL_USER_REQUEST"
rm -f "$REPORT"

clear_install_user_request() {
    bash "$INSTALL_USER_REQUEST" clear >/dev/null 2>&1 || true
}
trap clear_install_user_request EXIT

echo "=== Installing exact shared package ==="
bash "$INSTALL_USER_REQUEST" write admin
sudo /usr/sbin/installer -pkg "$PKG" -target /
clear_install_user_request

echo "=== Verifying package receipt and app bundle ==="
RECEIPT=$(/usr/sbin/pkgutil --pkg-info com.capsem.pkg)
printf '%s\n' "$RECEIPT"
printf '%s\n' "$RECEIPT" | grep -Fx "version: $VERSION"
test -d "/Applications/Capsem.app"
APP_VERSION=$(/usr/libexec/PlistBuddy \
    -c 'Print :CFBundleShortVersionString' \
    "/Applications/Capsem.app/Contents/Info.plist")
test "$APP_VERSION" = "$VERSION"

echo "=== Verifying installed binary cohort ==="
for binary in "${BINARIES[@]}"; do
    path="$CAPSEM_BIN_DIR/$binary"
    test -x "$path"
    "$path" --version | grep -F "$VERSION"
done

verify_channel() {
    local channel="$1"
    local manifest_url="$2"
    python3 "$VERIFY" \
        --capsem "$CAPSEM" \
        --manifest-url "$manifest_url" \
        --channel "$channel" \
        --package-version "$VERSION"
}

echo "=== Verifying initially installed channel ==="
verify_channel "$CHANNEL" "$MANIFEST_URL"

echo "=== Final installed-product status ==="
STATUS=$(capsem status)
printf '%s\n' "$STATUS"

python3 - "$REPORT" "$VERSION" "$CHANNEL" \
    "$APP_VERSION" "$(uname -r)" "$(uname -m)" <<'PY'
import json
from pathlib import Path
import sys

report = {
    "schema": "capsem.macos_tart_glowup.v1",
    "package_version": sys.argv[2],
    "initial_channel": sys.argv[3],
    "app_version": sys.argv[4],
    "guest_kernel": sys.argv[5],
    "guest_arch": sys.argv[6],
    "package_receipt": True,
    "app_bundle": True,
    "binary_cohort": True,
    "installed_status": True,
}
Path(sys.argv[1]).write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
PY

echo "Tart macOS installed-package glow-up passed"
