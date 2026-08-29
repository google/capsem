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
TRANSITION_SUPPORT="$SHARE/macos_tart_transition_support.py"
INSTALL_USER_REQUEST="$SHARE/macos-install-user-request.sh"
INSTALL_MANIFEST_REQUEST="$SHARE/install-manifest-request.sh"
REPORT="$SHARE/report.json"
INSTALLED_EVIDENCE="$SHARE/installed-evidence.json"
FRESH_INSTALLED_EVIDENCE="$SHARE/fresh-installed-evidence.json"
PRESERVED_INSTALLED_EVIDENCE="$SHARE/preserved-installed-evidence.json"
FRESH_TRANSITION_EVIDENCE="$SHARE/fresh-transition-evidence.json"
UPDATE_TRANSITION_EVIDENCE="$SHARE/update-transition-evidence.json"
TAMPER_REJECTION_EVIDENCE="$SHARE/tamper-rejection-evidence.json"
INCOMPATIBLE_REJECTION_EVIDENCE="$SHARE/incompatible-rejection-evidence.json"
ORIGINAL_MANIFEST="$SHARE/original-manifest.json"
UPDATED_MANIFEST="$SHARE/updated-manifest.json"
TAMPERED_MANIFEST="$SHARE/tampered-manifest.json"
INCOMPATIBLE_MANIFEST="$SHARE/incompatible-manifest.json"
REMOTE_MANIFEST="$SHARE/candidate/assets/$CHANNEL/manifest.json"
INSTALLED_MANIFEST="$CAPSEM_HOME/assets/manifest.json"
INSTALLED_METADATA="$CAPSEM_HOME/assets/manifest-metadata.json"
MANIFEST_BEFORE_REJECTION="$SHARE/manifest-before-rejection.json"
METADATA_BEFORE_REJECTION="$SHARE/manifest-metadata-before-rejection.json"
SERVICE_LOG_DIR="$CAPSEM_HOME/run"
SERVICE_PLIST="$HOME/Library/LaunchAgents/com.capsem.service.plist"
SERVICE_PLIST_BACKUP="$SHARE/com.capsem.service.plist.before-glowup"
RELEASE_HTTP_PORT=18765
RELEASE_HTTP_LOG="$SHARE/release-http.log"
RELEASE_HTTP_READY="$SHARE/release-http-ready.json"
RELEASE_HTTP_PID=""
# shellcheck source=build_system/packaging/macos/macos-tart-regression-probes.sh
source "$SHARE/macos-tart-regression-probes.sh"
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
test -f "$TRANSITION_SUPPORT"
test -f "$INSTALL_USER_REQUEST"
test -f "$INSTALL_MANIFEST_REQUEST"
test -f "$SHARE/macos-tart-regression-probes.sh"
test -s "$ORIGINAL_MANIFEST"
test -s "$UPDATED_MANIFEST"
test -s "$TAMPERED_MANIFEST"
test -s "$INCOMPATIBLE_MANIFEST"
test -s "$REMOTE_MANIFEST"
cmp -s "$ORIGINAL_MANIFEST" "$REMOTE_MANIFEST"
test "$(python3 "$TRANSITION_SUPPORT" sha256 "$ORIGINAL_MANIFEST")" != \
    "$(python3 "$TRANSITION_SUPPORT" sha256 "$UPDATED_MANIFEST")"
rm -f "$REPORT"
rm -f "$INSTALLED_EVIDENCE"
rm -f "$FRESH_INSTALLED_EVIDENCE"
rm -f "$PRESERVED_INSTALLED_EVIDENCE"
rm -f "$FRESH_TRANSITION_EVIDENCE" "$UPDATE_TRANSITION_EVIDENCE"
rm -f "$TAMPER_REJECTION_EVIDENCE" "$INCOMPATIBLE_REJECTION_EVIDENCE"
rm -f "$ASSET_HYDRATION_EVIDENCE" "$STALE_HELPER_EVIDENCE"
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
    rm -f "$RELEASE_HTTP_READY"
    python3 "$SHARE/serve-release-test-root.py" \
        --root "$SHARE" \
        --ready-file "$RELEASE_HTTP_READY" \
        --port "$RELEASE_HTTP_PORT" \
        >"$RELEASE_HTTP_LOG" 2>&1 &
    RELEASE_HTTP_PID=$!
    for attempt in $(seq 1 30); do
        if test -s "$RELEASE_HTTP_READY" && \
            python3 "$TRANSITION_SUPPORT" assert-url \
                "$MANIFEST_URL" "$ORIGINAL_MANIFEST"
        then
            return 0
        fi
        sleep 1
    done
    cat "$RELEASE_HTTP_LOG" >&2 || true
    return 1
}

observe_update_transition() {
    local kind="$1" result="$2" candidate_manifest_sha="$3"
    local after_line="$4" evidence_out="$5"
    local previous_manifest_sha="${6:-}"
    local command=(
        python3 "$SHARE/release_transition.py"
        --audit-log "$CAPSEM_HOME/logs/update.log" --after-line "$after_line"
        --kind "$kind" --result "$result" --source "$MANIFEST_URL"
        --candidate-manifest-sha256 "$candidate_manifest_sha" --timeout-seconds 180
        --evidence-out "$evidence_out"
    )
    if test -n "$previous_manifest_sha"; then
        command+=(--previous-manifest-sha256 "$previous_manifest_sha")
    fi
    "${command[@]}"
}
start_release_http_server

echo "=== Installing exact shared package ==="
bash "$INSTALL_USER_REQUEST" write admin
bash "$INSTALL_MANIFEST_REQUEST" write "$REMOTE_MANIFEST" "$MANIFEST_URL"
sudo /usr/sbin/installer -pkg "$PKG" -target /
clear_install_user_request
capsem_finish_install_hydration

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
verify_channel "$PACKAGE_CHANNEL" "$MANIFEST_URL" "$FRESH_INSTALLED_EVIDENCE"
ORIGINAL_MANIFEST_SHA=$(shasum -a 256 "$ORIGINAL_MANIFEST" | cut -d' ' -f1)
observe_update_transition fresh_install activated "$ORIGINAL_MANIFEST_SHA" 0 \
    "$FRESH_TRANSITION_EVIDENCE"

echo "=== Proving asynchronous visible asset repair ==="
capsem_probe_asset_hydration

echo "=== Reinstalling over a live native helper cohort ==="
capsem_probe_stale_helper_replacement
verify_binary_cohort

profile_tree_digest() {
    python3 "$TRANSITION_SUPPORT" tree-digest "$CAPSEM_HOME/profiles"
}

audit_line() {
    test -f "$CAPSEM_HOME/logs/update.log" \
        && wc -l < "$CAPSEM_HOME/logs/update.log" || printf '0\n'
}

promote_candidate() {
    python3 "$TRANSITION_SUPPORT" promote "$1" "$REMOTE_MANIFEST"
    python3 "$TRANSITION_SUPPORT" assert-url "$MANIFEST_URL" "$1"
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

echo "=== Activating a distinct valid update through automatic polling ==="
configure_fast_service_polling
UPDATE_AUDIT_FIRST_LINE=$(audit_line)
promote_candidate "$UPDATED_MANIFEST"
launchctl kickstart -k "gui/$(id -u)/com.capsem.service"
UPDATED_MANIFEST_SHA=$(python3 "$TRANSITION_SUPPORT" sha256 "$UPDATED_MANIFEST")
observe_update_transition profile_only activated "$UPDATED_MANIFEST_SHA" \
    "$UPDATE_AUDIT_FIRST_LINE" "$UPDATE_TRANSITION_EVIDENCE"
verify_channel "$CHANNEL" "$MANIFEST_URL" "$INSTALLED_EVIDENCE"

echo "=== Capturing the activated state before rejection candidates ==="
cp "$INSTALLED_MANIFEST" "$MANIFEST_BEFORE_REJECTION"
cp "$INSTALLED_METADATA" "$METADATA_BEFORE_REJECTION"
PROFILE_DIGEST_BEFORE=$(profile_tree_digest)
mkdir -p "$SERVICE_LOG_DIR"
PREVIOUS_MANIFEST_SHA=$(python3 "$TRANSITION_SUPPORT" sha256 "$MANIFEST_BEFORE_REJECTION")

assert_activated_state_preserved() {
    cmp -s "$MANIFEST_BEFORE_REJECTION" "$INSTALLED_MANIFEST"
    cmp -s "$METADATA_BEFORE_REJECTION" "$INSTALLED_METADATA"
    test "$(profile_tree_digest)" = "$PROFILE_DIGEST_BEFORE"
    /usr/sbin/pkgutil --pkg-info com.capsem.pkg | grep -Fx "version: $VERSION"
    verify_binary_cohort
}

reject_candidate() {
    local kind="$1" candidate="$2" evidence="$3"
    local marker candidate_sha
    marker=$(audit_line)
    promote_candidate "$candidate"
    launchctl kickstart -k "gui/$(id -u)/com.capsem.service"
    candidate_sha=$(python3 "$TRANSITION_SUPPORT" sha256 "$candidate")
    observe_update_transition "$kind" rejected "$candidate_sha" \
        "$marker" "$evidence" "$PREVIOUS_MANIFEST_SHA"
    assert_activated_state_preserved
}

echo "=== Rejecting tampered and incompatible candidates ==="
reject_candidate tampered_artifact "$TAMPERED_MANIFEST" "$TAMPER_REJECTION_EVIDENCE"
reject_candidate incompatible_profile "$INCOMPATIBLE_MANIFEST" \
    "$INCOMPATIBLE_REJECTION_EVIDENCE"

echo "=== Restoring the exact source and reproving the installed product ==="
promote_candidate "$UPDATED_MANIFEST"
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

python3 "$TRANSITION_SUPPORT" write-report \
    --output "$REPORT" --installed "$INSTALLED_EVIDENCE" \
    --fresh-installed "$FRESH_INSTALLED_EVIDENCE" \
    --preserved "$PRESERVED_INSTALLED_EVIDENCE" \
    --fresh-transition "$FRESH_TRANSITION_EVIDENCE" \
    --update-transition "$UPDATE_TRANSITION_EVIDENCE" \
    --tamper-rejection "$TAMPER_REJECTION_EVIDENCE" \
    --incompatible-rejection "$INCOMPATIBLE_REJECTION_EVIDENCE" \
    --asset-hydration "$ASSET_HYDRATION_EVIDENCE" \
    --stale-helper "$STALE_HELPER_EVIDENCE" \
    --package "$PKG" --app-version "$APP_VERSION" \
    --kernel "$(uname -r)" --architecture "$(uname -m)"

echo "Tart macOS installed-package glow-up passed"
