"""The replacement service can be ready before PackageKit publishes its receipt."""

import os
import subprocess
from pathlib import Path

import pytest

SCRIPT = Path(__file__).resolve().parents[2] / "packaging/macos/macos-tart-binary-apply.sh"


@pytest.mark.parametrize("settles", [True, False])
def test_pair_verification_observes_exact_package_receipt(tmp_path, settles):
    shell = r'''
set -euo pipefail
source "$SOURCE"
SHARE="$WORK"
CAPSEM=fixture_capsem
VERIFY=fixture-verifier
MANIFEST_URL=http://fixture.invalid/manifest.json
CHANNEL=nightly
capsem_pair_field() {
    case "$*" in
        *"--field version") echo 0.7.1 ;;
        *"--field scope") echo binary_apply_only ;;
        *) echo fixture ;;
    esac
}
fixture_capsem() { echo 'capsem 0.7.1'; }
verify_binary_cohort() { :; }
python3() {
    case "$*" in
        *" binaries "*) echo capsem ;;
    esac
}
function /usr/sbin/pkgutil {
    local count=0
    if test -f "$WORK/queries"; then read -r count < "$WORK/queries"; fi
    count=$((count + 1))
    echo "$count" > "$WORK/queries"
    if test "$SETTLES" = yes && test "$count" -ge 2; then
        echo 'version: 0.7.1'
    else
        echo 'version: 0.6.6'
    fi
}
function /usr/libexec/PlistBuddy { echo 0.7.1; }
sleep() { :; }
capsem_pair_verify after
'''
    result = subprocess.run(
        ["bash", "-c", shell], capture_output=True, text=True, timeout=10,
        env={**os.environ, "SOURCE": str(SCRIPT), "WORK": str(tmp_path),
             "SETTLES": "yes" if settles else "no"},
    )
    queries = int((tmp_path / "queries").read_text())
    if settles:
        assert result.returncode == 0, result.stderr
        assert queries == 2
    else:
        assert result.returncode != 0
        assert queries == 90
