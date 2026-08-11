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
INSTALL_MANIFEST_REQUEST="$SHARE/install-manifest-request.sh"
REPORT="$SHARE/report.json"
INSTALLED_EVIDENCE="$SHARE/installed-evidence.json"
PRESERVED_INSTALLED_EVIDENCE="$SHARE/preserved-installed-evidence.json"
ORIGINAL_MANIFEST="$SHARE/original-manifest.json"
TAMPERED_MANIFEST="$SHARE/tampered-manifest.json"
REMOTE_MANIFEST="$SHARE/candidate/assets/$CHANNEL/manifest.json"
INSTALLED_MANIFEST="$CAPSEM_HOME/assets/manifest.json"
INSTALLED_METADATA="$CAPSEM_HOME/assets/manifest-metadata.json"
MANIFEST_BEFORE_REJECTION="$SHARE/manifest-before-rejection.json"
METADATA_BEFORE_REJECTION="$SHARE/manifest-metadata-before-rejection.json"
SERVICE_LOG_DIR="$CAPSEM_HOME/run"
# `service.log` names a daily-rotated stream, so the bare name is an empty
# file the moment the service has rotated. Reading it directly polled
# nothing for three minutes while the rejection this proof waits for sat in
# `service.<date>.log`, and reported a service that had not rejected the
# tampered manifest when it had.
service_log_stream() {
    cat "$SERVICE_LOG_DIR"/service*.log 2>/dev/null || true
}
SERVICE_PLIST="$HOME/Library/LaunchAgents/com.capsem.service.plist"
SERVICE_PLIST_BACKUP="$SHARE/com.capsem.service.plist.before-glowup"
RELEASE_HTTP_PORT=18765
RELEASE_HTTP_LOG="$SHARE/release-http.log"
RELEASE_HTTP_PID=""
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
    capsem-mock-server
    capsem-bench-rs
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
test -f "$INSTALL_MANIFEST_REQUEST"
test -s "$ORIGINAL_MANIFEST"
test -s "$TAMPERED_MANIFEST"
test -s "$REMOTE_MANIFEST"
cmp -s "$ORIGINAL_MANIFEST" "$REMOTE_MANIFEST"
if cmp -s "$ORIGINAL_MANIFEST" "$TAMPERED_MANIFEST"; then
    echo "ERROR: tampered manifest is byte-identical to the original" >&2
    exit 1
fi
rm -f "$REPORT"
rm -f "$INSTALLED_EVIDENCE"
rm -f "$PRESERVED_INSTALLED_EVIDENCE"

echo "=== Verifying clean guest precondition ==="
if /usr/sbin/pkgutil --pkg-info com.capsem.pkg >/dev/null 2>&1; then
    echo "ERROR: Tart base image already has the Capsem package receipt" >&2
    exit 1
fi
test ! -e "/Applications/Capsem.app"
test ! -e "$CAPSEM_HOME"

clear_install_user_request() {
    bash "$INSTALL_USER_REQUEST" clear >/dev/null 2>&1 || true
    bash "$INSTALL_MANIFEST_REQUEST" clear >/dev/null 2>&1 || true
}
cleanup_guest() {
    clear_install_user_request
    if test -s "$ORIGINAL_MANIFEST" && test -d "$(dirname "$REMOTE_MANIFEST")"; then
        cp "$ORIGINAL_MANIFEST" "$REMOTE_MANIFEST" || true
    fi
    if test -s "$SERVICE_PLIST_BACKUP"; then
        cp "$SERVICE_PLIST_BACKUP" "$SERVICE_PLIST" || true
        launchctl bootout "gui/$(id -u)" "$SERVICE_PLIST" >/dev/null 2>&1 || true
        launchctl bootstrap "gui/$(id -u)" "$SERVICE_PLIST" >/dev/null 2>&1 || true
    fi
    if test -n "$RELEASE_HTTP_PID"; then
        kill "$RELEASE_HTTP_PID" >/dev/null 2>&1 || true
        wait "$RELEASE_HTTP_PID" >/dev/null 2>&1 || true
    fi
}
trap cleanup_guest EXIT

start_release_http_server() {
    local expected_url="http://127.0.0.1:${RELEASE_HTTP_PORT}/candidate/assets/${CHANNEL}/manifest.json"
    if test "$MANIFEST_URL" != "$expected_url"; then
        echo "ERROR: manifest polling URL must be $expected_url (got: $MANIFEST_URL)" >&2
        exit 1
    fi
    python3 -m http.server "$RELEASE_HTTP_PORT" \
        --bind 127.0.0.1 \
        --directory "$SHARE" \
        >"$RELEASE_HTTP_LOG" 2>&1 &
    RELEASE_HTTP_PID=$!
    for attempt in $(seq 1 30); do
        if python3 - "$MANIFEST_URL" "$ORIGINAL_MANIFEST" <<'PY'
from pathlib import Path
import sys
import urllib.request

try:
    with urllib.request.urlopen(sys.argv[1], timeout=2) as response:
        fetched = response.read()
except OSError:
    raise SystemExit(1)
if fetched != Path(sys.argv[2]).read_bytes():
    raise SystemExit(1)
PY
        then
            return 0
        fi
        sleep 1
    done
    cat "$RELEASE_HTTP_LOG" >&2 || true
    return 1
}

start_release_http_server

echo "=== Installing exact shared package ==="
bash "$INSTALL_USER_REQUEST" write admin
bash "$INSTALL_MANIFEST_REQUEST" write "$REMOTE_MANIFEST" "$MANIFEST_URL"
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
verify_binary_cohort() {
    local binary
    local path
    for binary in "${BINARIES[@]}"; do
        path="$CAPSEM_BIN_DIR/$binary"
        test -x "$path"
        "$path" --version | grep -F "$VERSION"
        codesign --verify --strict "$path"
        codesign -d --verbose=4 "$path" 2>&1 \
            | grep -F "Signature=adhoc"
    done
}
verify_binary_cohort
read -r PACKAGE_CHANNEL PACKAGE_MANIFEST_URL < <(python3 -c 'import json,sys; m=json.load(open(sys.argv[1])); print(m["channel"],m["manifest_url"])' "$INSTALLED_METADATA")
verify_channel() {
    local channel="$1"
    local manifest_url="$2"
    local evidence_out="$3"
    python3 "$VERIFY" \
        --capsem "$CAPSEM" \
        --manifest-url "$manifest_url" \
        --metadata-manifest-url "$PACKAGE_MANIFEST_URL" \
        --channel "$channel" \
        --package-version "$VERSION" \
        --artifact "$PKG" \
        --platform macos \
        --architecture arm64 \
        --evidence-out "$evidence_out"
}
echo "=== Verifying initially installed channel ==="
verify_channel "$PACKAGE_CHANNEL" "$MANIFEST_URL" "$INSTALLED_EVIDENCE"

profile_tree_digest() {
    python3 - "$CAPSEM_HOME/profiles" <<'PY'
import hashlib
from pathlib import Path
import sys

root = Path(sys.argv[1])
digest = hashlib.sha256()
for path in sorted(candidate for candidate in root.rglob("*") if candidate.is_file()):
    digest.update(path.relative_to(root).as_posix().encode())
    digest.update(b"\0")
    digest.update(hashlib.sha256(path.read_bytes()).digest())
print(digest.hexdigest())
PY
}

wait_for_automatic_rejection() {
    local first_line="$1"
    local attempt
    for attempt in $(seq 1 90); do
        if service_log_stream | tail -n "+$first_line" \
            | grep -Fq "automatic release update failed"; then
            return 0
        fi
        sleep 2
    done
    service_log_stream | tail -n "+$first_line" >&2 || true
    launchctl print "gui/$(id -u)/com.capsem.service" >&2 || true
    return 1
}

configure_fast_service_polling() {
    test -s "$SERVICE_PLIST"
    cp "$SERVICE_PLIST" "$SERVICE_PLIST_BACKUP"
    /usr/libexec/PlistBuddy \
        -c "Add :EnvironmentVariables:CAPSEM_AUTOMATIC_UPDATE_INITIAL_DELAY_SECS string 2" \
        "$SERVICE_PLIST" \
        || /usr/libexec/PlistBuddy \
            -c "Set :EnvironmentVariables:CAPSEM_AUTOMATIC_UPDATE_INITIAL_DELAY_SECS 2" \
            "$SERVICE_PLIST"
    /usr/libexec/PlistBuddy \
        -c "Add :EnvironmentVariables:CAPSEM_AUTOMATIC_UPDATE_POLL_SECS string 2" \
        "$SERVICE_PLIST" \
        || /usr/libexec/PlistBuddy \
            -c "Set :EnvironmentVariables:CAPSEM_AUTOMATIC_UPDATE_POLL_SECS 2" \
            "$SERVICE_PLIST"
    launchctl bootout "gui/$(id -u)" "$SERVICE_PLIST"
    launchctl bootstrap "gui/$(id -u)" "$SERVICE_PLIST"
    launchctl kickstart -k "gui/$(id -u)/com.capsem.service"
    launchctl print "gui/$(id -u)/com.capsem.service" \
        | grep -F "CAPSEM_AUTOMATIC_UPDATE_INITIAL_DELAY_SECS"
    launchctl print "gui/$(id -u)/com.capsem.service" \
        | grep -F "CAPSEM_AUTOMATIC_UPDATE_POLL_SECS"
}

echo "=== Rejecting a tampered manifest through automatic polling ==="
cp "$INSTALLED_MANIFEST" "$MANIFEST_BEFORE_REJECTION"
cp "$INSTALLED_METADATA" "$METADATA_BEFORE_REJECTION"
PROFILE_DIGEST_BEFORE=$(profile_tree_digest)
mkdir -p "$SERVICE_LOG_DIR"
SERVICE_LOG_FIRST_LINE=$(( $(service_log_stream | wc -l) + 1 ))
cp "$TAMPERED_MANIFEST" "$REMOTE_MANIFEST"
cmp -s "$TAMPERED_MANIFEST" "$REMOTE_MANIFEST"
python3 - "$MANIFEST_URL" "$TAMPERED_MANIFEST" <<'PY'
from pathlib import Path
import sys
import urllib.request

with urllib.request.urlopen(sys.argv[1], timeout=30) as response:
    fetched = response.read()
expected = Path(sys.argv[2]).read_bytes()
if fetched != expected:
    raise SystemExit("polled manifest URL did not expose the tampered candidate")
PY
configure_fast_service_polling
wait_for_automatic_rejection "$SERVICE_LOG_FIRST_LINE"

echo "=== Proving the exact installed state survived rejection ==="
cmp -s "$MANIFEST_BEFORE_REJECTION" "$INSTALLED_MANIFEST"
cmp -s "$METADATA_BEFORE_REJECTION" "$INSTALLED_METADATA"
test "$(profile_tree_digest)" = "$PROFILE_DIGEST_BEFORE"
RECEIPT_AFTER=$(/usr/sbin/pkgutil --pkg-info com.capsem.pkg)
printf '%s\n' "$RECEIPT_AFTER" | grep -Fx "version: $VERSION"
test "$(/usr/libexec/PlistBuddy \
    -c 'Print :CFBundleShortVersionString' \
    "/Applications/Capsem.app/Contents/Info.plist")" = "$VERSION"
verify_binary_cohort

echo "=== Restoring the exact source and reproving the installed product ==="
cp "$ORIGINAL_MANIFEST" "$REMOTE_MANIFEST"
launchctl kickstart -k "gui/$(id -u)/com.capsem.service"
for attempt in $(seq 1 60); do
    STATUS_OUTPUT=$("$CAPSEM" status 2>/dev/null || true)
    if grep -Fq "Service:   ok" <<<"$STATUS_OUTPUT"; then
        break
    fi
    if test "$attempt" = 60; then
        printf '%s\n' "$STATUS_OUTPUT" >&2
        exit 1
    fi
    sleep 2
done
verify_channel "$CHANNEL" "$MANIFEST_URL" "$PRESERVED_INSTALLED_EVIDENCE"

echo "=== Final installed-product status ==="
STATUS=$(capsem status)
printf '%s\n' "$STATUS"

python3 - "$REPORT" "$INSTALLED_EVIDENCE" "$PRESERVED_INSTALLED_EVIDENCE" \
    "$PKG" "$APP_VERSION" "$(uname -r)" "$(uname -m)" <<'PY'
import hashlib
import json
from pathlib import Path
import sys

installed = json.loads(Path(sys.argv[2]).read_text())
installed["package_receipt"] = True
installed["binary_cohort"] = True
preserved = json.loads(Path(sys.argv[3]).read_text())
preserved["package_receipt"] = True
preserved["binary_cohort"] = True
package_sha256 = hashlib.sha256(Path(sys.argv[4]).read_bytes()).hexdigest()
report = {
    "schema": "capsem.release_glowup.guest.v1",
    "artifact_sha256": package_sha256,
    "installed": installed,
    "preserved_installed": preserved,
    "tamper_rejection": {
        "schema": "capsem.installed_rejection.v1",
        "kind": "tampered_artifact",
        "result": "rejected",
        "preserved_previous": True,
        "manifest_unchanged": True,
        "manifest_metadata_unchanged": True,
        "profiles_unchanged": True,
        "package_unchanged": True,
        "service": "ok",
        "gateway": "ok",
    },
    "guest": {
        "app_version": sys.argv[5],
        "kernel": sys.argv[6],
        "architecture": sys.argv[7],
        "clean_precondition": True,
        "app_bundle": True,
        "installed_binary_signature": "ad-hoc",
    },
}
Path(sys.argv[1]).write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
PY

echo "Tart macOS installed-package glow-up passed"
