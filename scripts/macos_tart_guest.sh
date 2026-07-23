#!/bin/bash
# Install and exercise the shared Capsem package inside a clean Tart macOS VM.
set -euo pipefail

VERSION="${1:?usage: macos_tart_guest.sh VERSION MANIFEST_URL CHANNEL PACKAGE}"
MANIFEST_URL="${2:?missing manifest URL}"
CHANNEL="${3:?missing channel}"
PKG="${4:?missing exact package path}"
SHARE="/Volumes/My Shared Files/capsem-release"
CAPSEM_HOME="$HOME/.capsem"
CAPSEM_BIN_DIR="$CAPSEM_HOME/bin"
CAPSEM="$CAPSEM_BIN_DIR/capsem"
VERIFY="$SHARE/verify-installed-release.py"
INSTALL_USER_REQUEST="$SHARE/macos-install-user-request.sh"
REPORT="$SHARE/report.json"
INSTALLED_EVIDENCE="$SHARE/installed-evidence.json"
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
rm -f "$INSTALLED_EVIDENCE"

echo "=== Verifying clean guest precondition ==="
if /usr/sbin/pkgutil --pkg-info com.capsem.pkg >/dev/null 2>&1; then
    echo "ERROR: Tart base image already has the Capsem package receipt" >&2
    exit 1
fi
test ! -e "/Applications/Capsem.app"
test ! -e "$CAPSEM_HOME"

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
    codesign --verify --strict "$path"
    codesign -d --verbose=4 "$path" 2>&1 \
        | grep -F "Authority=Developer ID Application:"
done

verify_channel() {
    local channel="$1"
    local manifest_url="$2"
    python3 "$VERIFY" \
        --capsem "$CAPSEM" \
        --manifest-url "$manifest_url" \
        --channel "$channel" \
        --package-version "$VERSION" \
        --artifact "$PKG" \
        --platform macos \
        --architecture arm64 \
        --evidence-out "$INSTALLED_EVIDENCE"
}

echo "=== Verifying initially installed channel ==="
verify_channel "$CHANNEL" "$MANIFEST_URL"

echo "=== Final installed-product status ==="
STATUS=$(capsem status)
printf '%s\n' "$STATUS"

python3 - "$REPORT" "$INSTALLED_EVIDENCE" "$PKG" \
    "$APP_VERSION" "$(uname -r)" "$(uname -m)" <<'PY'
import hashlib
import json
from pathlib import Path
import sys

installed = json.loads(Path(sys.argv[2]).read_text())
installed["package_receipt"] = True
installed["binary_cohort"] = True
package_sha256 = hashlib.sha256(Path(sys.argv[3]).read_bytes()).hexdigest()
report = {
    "schema": "capsem.release_glowup.guest.v1",
    "artifact_sha256": package_sha256,
    "installed": installed,
    "guest": {
        "app_version": sys.argv[4],
        "kernel": sys.argv[5],
        "architecture": sys.argv[6],
        "clean_precondition": True,
        "app_bundle": True,
    },
}
Path(sys.argv[1]).write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
PY

echo "Tart macOS installed-package glow-up passed"
