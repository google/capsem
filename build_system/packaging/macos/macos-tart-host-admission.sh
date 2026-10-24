#!/bin/bash
# Sourced ONLY by the same ordinary Tart owner with independently bound candidate.

capsem_host_admission() {
    python3 -m capsem_builder.release.tools.release_admission_refusal "$@" \
        --root "$SHARE" --home "$CAPSEM_HOME" --maximum "$ADMISSION_MAXIMUM"
}

capsem_prove_host_admission_refusal() {
    export PYTHONPATH="$SHARE"
    local candidate status
    read -r candidate ADMISSION_MAXIMUM < <(python3 - "$SHARE" <<'PY'
import json
from pathlib import Path
import sys

root = Path(sys.argv[1])
spec = json.loads((root / "host-admission-inputs.json").read_text())
print(spec["candidate_package"], spec["maximum"])
PY
)
    BINARIES=()
    while IFS= read -r binary; do BINARIES+=("$binary"); done < <(capsem_host_admission binaries)
    test "${#BINARIES[@]}" -gt 0
    verify_binary_cohort
    /usr/sbin/pkgutil --pkg-info com.capsem.pkg | grep -Fx "version: $VERSION"
    test "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' /Applications/Capsem.app/Contents/Info.plist)" = "$VERSION"
    capsem_verify_host_before "$SHARE/host-refusal-before-installed.json"
    printf 'original host admission preservation proof\n' > "$CAPSEM_HOME/preinstall-preservation.txt"
    capsem_host_admission snapshot
    bash "$INSTALL_USER_REQUEST" write admin
    status=0
    sudo /usr/sbin/installer -pkg "$SHARE/$candidate" -target / \
        > "$SHARE/host-refusal-installer.log" 2>&1 || status=$?
    printf '%s\n' "$status" > "$SHARE/host-refusal-status.txt"
    clear_install_user_request
    test "$status" -ne 0
    # No kickstart, stop, restart, hydration or second installer attempt here.
    verify_binary_cohort
    /usr/sbin/pkgutil --pkg-info com.capsem.pkg | grep -Fx "version: $VERSION"
    capsem_verify_host_before "$SHARE/host-refusal-after-installed.json"
    capsem_host_admission finish
}

capsem_verify_host_before() {
    python3 "$VERIFY" --capsem "$CAPSEM" --manifest-url "$MANIFEST_URL" \
        --metadata-manifest-url "$MANIFEST_URL" --channel "$CHANNEL" \
        --package-version "$VERSION" --artifact "$PKG" --platform macos \
        --architecture arm64 --evidence-out "$1"
}
