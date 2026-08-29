#!/bin/bash
# Boot a real Capsem guest on the physical Mac using the exact .pkg payload.
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
PKG=""
VERSION=""
ASSETS_DIR=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        --package)
            PKG="${2:?--package requires a value}"
            shift 2
            ;;
        --version)
            VERSION="${2:?--version requires a value}"
            shift 2
            ;;
        --assets-dir)
            ASSETS_DIR="${2:?--assets-dir requires a value}"
            shift 2
            ;;
        *)
            echo "usage: $0 --package PKG --version VERSION --assets-dir DIR" >&2
            exit 2
            ;;
    esac
done
[ -n "$PKG" ] && [ -n "$VERSION" ] && [ -n "$ASSETS_DIR" ] || {
    echo "ERROR: package, version, and selected assets are required" >&2
    exit 2
}
WORK_ROOT="$ROOT/target/macos-package-boot"
EXPANDED="$WORK_ROOT/expanded"
CAPSEM_HOME_DIR="$WORK_ROOT/home"
RUN_DIR=$(mktemp -d /tmp/capsem-pkg-boot.XXXXXX)
DOCTOR_LOG="$WORK_ROOT/doctor.log"
DOCTOR_EVIDENCE="$WORK_ROOT/doctor.json"
WINTERFELL_LOG="$WORK_ROOT/winterfell.log"
WINTERFELL_EVIDENCE="$WORK_ROOT/winterfell.json"
PERSISTENT_PIN_EVIDENCE="$WORK_ROOT/persistent-pin-resume.json"

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
capsem_isolated() {
    CAPSEM_HOME="$CAPSEM_HOME_DIR" \
    CAPSEM_RUN_DIR="$RUN_DIR" \
    CAPSEM_ASSETS_DIR="$CAPSEM_HOME_DIR/assets" \
    CAPSEM_PROFILES_DIR="$CAPSEM_HOME_DIR/profiles" \
        "$CAPSEM_HOME_DIR/bin/capsem" "$@"
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
        "$ASSETS_DIR" \
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
        --keep-session \
        --timeout 300

echo "=== Proving a named VM keeps its saved profile and image pins ==="
ACTIVE_PROFILE=$(find "$RUN_DIR/persistent" -type f -name active_profile.toml -print -quit)
test -s "$ACTIVE_PROFILE"
PIN_SHA_BEFORE=$(shasum -a 256 "$ACTIVE_PROFILE" | cut -d' ' -f1)
capsem_isolated suspend macos-package-vm-boot
capsem_isolated stop
CURRENT_PROFILE="$CAPSEM_HOME_DIR/profiles/code"
CURRENT_PROFILE_BACKUP="$WORK_ROOT/code-profile-before-selection-change"
cp -R "$CURRENT_PROFILE" "$CURRENT_PROFILE_BACKUP"
python3 - "$CURRENT_PROFILE/profile.toml" <<'PY'
from pathlib import Path
import re
import sys

path = Path(sys.argv[1])
updated, count = re.subn(
    r'(?m)^revision\s*=\s*"[^"]+"$',
    'revision = "glowup-current-profile-advanced"',
    path.read_text(),
    count=1,
)
if count != 1:
    raise SystemExit("installed code profile has no revision to advance")
path.write_text(updated)
PY
capsem_isolated start
capsem_isolated resume macos-package-vm-boot
for attempt in $(seq 1 90); do
    if capsem_isolated exec macos-package-vm-boot \
        'printf CAPSEM_PERSISTENT_PIN_RESUME_OK' \
        | grep -Fq CAPSEM_PERSISTENT_PIN_RESUME_OK
    then
        break
    fi
    if [ "$attempt" -eq 90 ]; then
        capsem_isolated info macos-package-vm-boot --json >&2 || true
        echo "ERROR: saved-profile persistent VM did not resume" >&2
        exit 1
    fi
    sleep 2
done
PIN_SHA_AFTER=$(shasum -a 256 "$ACTIVE_PROFILE" | cut -d' ' -f1)
test "$PIN_SHA_AFTER" = "$PIN_SHA_BEFORE"
capsem_isolated suspend macos-package-vm-boot
capsem_isolated stop
rm -rf "$CURRENT_PROFILE"
mv "$CURRENT_PROFILE_BACKUP" "$CURRENT_PROFILE"
capsem_isolated start
capsem_isolated delete macos-package-vm-boot
python3 - "$PERSISTENT_PIN_EVIDENCE" "$PIN_SHA_BEFORE" <<'PY'
import json
from pathlib import Path
import sys

Path(sys.argv[1]).write_text(
    json.dumps(
        {
            "schema": "capsem.persistent_pin_resume.v1",
            "persistent_pin_resume": True,
            "saved_profile_sha256": sys.argv[2],
        },
        indent=2,
        sort_keys=True,
    )
    + "\n"
)
PY

echo "=== Running full doctor from the exact package cohort ==="
set +e
CAPSEM_HOME="$CAPSEM_HOME_DIR" \
CAPSEM_RUN_DIR="$RUN_DIR" \
CAPSEM_ASSETS_DIR="$CAPSEM_HOME_DIR/assets" \
CAPSEM_PROFILES_DIR="$CAPSEM_HOME_DIR/profiles" \
    "$CAPSEM_HOME_DIR/bin/capsem" doctor >"$DOCTOR_LOG" 2>&1
DOCTOR_STATUS=$?
set -e
python3 - "$DOCTOR_EVIDENCE" "$DOCTOR_LOG" "$DOCTOR_STATUS" <<'PY'
import json
from pathlib import Path
import sys

status = int(sys.argv[3])
Path(sys.argv[1]).write_text(
    json.dumps(
        {
            "schema": "capsem.installed_doctor.v1",
            "passed": status == 0,
            "exit_code": status,
            "log": str(Path(sys.argv[2]).resolve()),
        },
        indent=2,
        sort_keys=True,
    )
    + "\n"
)
PY
if [ "$DOCTOR_STATUS" -ne 0 ]; then
    echo "ERROR: full installed Doctor failed; retained evidence: $DOCTOR_EVIDENCE" >&2
    cat "$DOCTOR_LOG" >&2
    exit "$DOCTOR_STATUS"
fi

echo "=== Running installed Winterfell from the exact package cohort ==="
set +e
CAPSEM_HOME="$CAPSEM_HOME_DIR" \
CAPSEM_RUN_DIR="$RUN_DIR" \
    uv run --project build_system --frozen python "$ROOT/scripts/run-installed-winterfell.py" \
        --bin-dir "$CAPSEM_HOME_DIR/bin" \
        --assets-dir "$CAPSEM_HOME_DIR/assets" \
        --profiles-dir "$CAPSEM_HOME_DIR/profiles" \
        --evidence-out "$WINTERFELL_EVIDENCE" \
        >"$WINTERFELL_LOG" 2>&1
WINTERFELL_STATUS=$?
set -e
if [ "$WINTERFELL_STATUS" -ne 0 ]; then
    echo "ERROR: installed Winterfell failed; retained evidence: $WINTERFELL_EVIDENCE" >&2
    cat "$WINTERFELL_LOG" >&2
    exit "$WINTERFELL_STATUS"
fi

python3 - "$WORK_ROOT/report.json" "$PKG" "$VERSION" \
    "$DOCTOR_EVIDENCE" "$WINTERFELL_EVIDENCE" "$PERSISTENT_PIN_EVIDENCE" <<'PY'
import hashlib
import json
from pathlib import Path
import sys

package = Path(sys.argv[2]).resolve()
doctor = json.loads(Path(sys.argv[4]).read_text())
winterfell = json.loads(Path(sys.argv[5]).read_text())
persistent_pin = json.loads(Path(sys.argv[6]).read_text())
if doctor.get("schema") != "capsem.installed_doctor.v1" or not doctor.get("passed"):
    raise SystemExit(f"full installed doctor evidence failed: {doctor}")
if winterfell.get("schema") != "capsem.installed_winterfell.v1" or not winterfell.get(
    "passed"
):
    raise SystemExit(f"installed Winterfell evidence failed: {winterfell}")
if persistent_pin.get("schema") != "capsem.persistent_pin_resume.v1" or not persistent_pin.get(
    "persistent_pin_resume"
):
    raise SystemExit(f"persistent pin resume evidence failed: {persistent_pin}")
report = {
    "schema": "capsem.macos_package_boot.v1",
    "package": str(package),
    "package_sha256": hashlib.sha256(package.read_bytes()).hexdigest(),
    "package_version": sys.argv[3],
    "package_payload_materialized": True,
    "session_created": True,
    "guest_vm_booted": True,
    "guest_shell_marker": "CAPSEM_MACOS_PACKAGE_VM_BOOT_OK",
    "full_doctor": True,
    "installed_winterfell": True,
    "persistent_pin_resume": True,
    "doctor_evidence": doctor,
    "winterfell_evidence": winterfell,
    "persistent_pin_evidence": persistent_pin,
}
Path(sys.argv[1]).write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
PY

echo "Exact macOS package guest-VM doctor/Winterfell proof passed: $WORK_ROOT/report.json"
