"""Exact helper source with labelled privilege/timeout boundary doubles.

Real files, decoding, native SHA tool and modes are exercised. The future
candidate executable is NEVER run, and these tests do not certify root/UID
switching or a particular Linux target's runtime dependencies.
"""

from __future__ import annotations

import shlex
import subprocess
from pathlib import Path

import pytest
from capsem_builder import crypto

ROOT = Path(__file__).resolve().parents[3]
HELPER = ROOT / "build_system/packaging/shared/preinstall-ledger"
PAYLOAD = b"future-candidate-source-double\n"


def invoke(
    root: Path,
    *,
    payload: bytes = PAYLOAD,
    expected: bytes = PAYLOAD,
    target: str = "fixture-user",
    current_uid: int = 0,
    target_uid: int = 1000,
    decode_status: int = 0,
    candidate_status: int = 0,
    timeout_status: int = 0,
    platform: str = "Linux",
) -> subprocess.CompletedProcess[str]:
    source = root / "source"
    source.write_bytes(payload)
    def q(value: object) -> str:
        return shlex.quote(str(value))
    # Absolute command functions label platform/privilege boundaries only.
    # SHA still executes the real OS implementation (shasum on macOS).
    script = f"""
set -euo pipefail
function /usr/bin/uname {{ echo {q(platform)}; }}
function /usr/bin/id {{ if [ "$#" -eq 1 ]; then echo {current_uid}; else echo {target_uid}; fi; }}
function /usr/bin/mktemp {{ /bin/mkdir -m 0700 {q(root / 'stage')}; printf '%s\\n' {q(root / 'stage')}; }}
function /usr/bin/sha256sum {{
    if [ -x /usr/bin/sha256sum ]; then command /usr/bin/sha256sum "$@"; else /usr/bin/shasum -a 256 "$@"; fi
}}
function /usr/bin/timeout {{
    printf '%s\\n' "$@" > {q(root / 'timeout-argv')}
    if [ {timeout_status} -ne 0 ]; then return {timeout_status}; fi
    shift 3
    "$@"
}}
function /usr/sbin/runuser {{
    printf '%s\\n' "$@" > {q(root / 'runuser-argv')}
    /bin/ls -ld "$4" "$(dirname "$4")" > {q(root / 'modes')}
    /bin/cat "$4" > {q(root / 'observed-bytes')}
    return {candidate_status}
}}
function /usr/bin/su {{
    printf '%s\\n' "$@" > {q(root / 'su-argv')}
    /bin/ls -ld "$5" "$(dirname "$5")" > {q(root / 'modes')}
    /bin/cat "$5" > {q(root / 'observed-bytes')}
    return {candidate_status}
}}
source {q(HELPER)}
capsem_write_preinstall_resource() {{ /bin/cat {q(source)}; return {decode_status}; }}
capsem_admit_preinstall_ledger {q(target)} {crypto.sha256(expected).hexdigest()} {len(expected)} 15 2
"""
    return subprocess.run(["bash", "-c", script], text=True, capture_output=True, timeout=10)


def test_verified_stage_executes_exact_future_mode_as_resolved_user_then_disappears(tmp_path: Path) -> None:
    result = invoke(tmp_path)
    assert result.returncode == 0, result.stderr
    assert (tmp_path / "observed-bytes").read_bytes() == PAYLOAD
    assert (tmp_path / "runuser-argv").read_text().splitlines() == [
        "-u", "fixture-user", "--", str(tmp_path / "stage/capsem-ledger"),
        "--preinstall-ledger-admission", "--target-user", "fixture-user",
    ]
    assert (tmp_path / "timeout-argv").read_text().splitlines()[:3] == [
        "--signal=TERM", "--kill-after=2s", "15s",
    ]
    modes = (tmp_path / "modes").read_text()
    assert "-r-xr-xr-x" in modes and "drwx--x--x" in modes
    assert not (tmp_path / "stage").exists()


@pytest.mark.parametrize(
    "changes",
    [
        {"payload": b"short"},
        {"payload": bytes(len(PAYLOAD))},
        {"decode_status": 1},
        {"current_uid": 501},
        {"target_uid": 0},
        {"target": "root"},
        {"target": "fixture;invalid"},
    ],
)
@pytest.mark.parametrize("platform", ["Linux", "Darwin"])
def test_invalid_binding_or_transport_never_reaches_candidate_and_cleans_stage(
    tmp_path: Path, changes: dict, platform: str,
) -> None:
    result = invoke(tmp_path, platform=platform, **changes)
    assert result.returncode != 0
    assert not (tmp_path / "runuser-argv").exists()
    assert not (tmp_path / "su-argv").exists()
    assert not (tmp_path / "stage").exists()


@pytest.mark.parametrize("status", [1, 23, 127])
def test_candidate_refusal_or_missing_runtime_propagates_and_cleans(tmp_path: Path, status: int) -> None:
    result = invoke(tmp_path, candidate_status=status)
    assert result.returncode == status, result.stderr
    assert not (tmp_path / "stage").exists()


def test_deadline_refusal_cleans_before_any_candidate_effect(tmp_path: Path) -> None:
    result = invoke(tmp_path, timeout_status=124)
    assert result.returncode == 124
    assert not (tmp_path / "runuser-argv").exists()
    assert not (tmp_path / "stage").exists()


@pytest.mark.parametrize("status", [0, 23])
def test_mac_exact_su_argv_identity_and_cleanup_without_gnu_timeout(tmp_path: Path, status: int) -> None:
    result = invoke(tmp_path, platform="Darwin", candidate_status=status)
    assert result.returncode == status, result.stderr
    assert (tmp_path / "su-argv").read_text().splitlines() == [
        "fixture-user", "-c", 'exec "$1" --preinstall-ledger-admission --target-user "$2"',
        "capsem-preinstall", str(tmp_path / "stage/capsem-ledger"), "fixture-user",
    ]
    assert (tmp_path / "observed-bytes").read_bytes() == PAYLOAD
    assert not (tmp_path / "timeout-argv").exists()
    assert not (tmp_path / "runuser-argv").exists()
    assert not (tmp_path / "stage").exists()


def test_admission_precedes_every_package_retirement_branch() -> None:
    preinst = (ROOT / "build_system/packaging/linux/deb-preinst.sh").read_text()
    admission = preinst.index('capsem_run_preinstall_ledger_admission "$TARGET_USER"')
    assert admission < preinst.index('CAPSEM_INSTALL_PHASE="stop_existing_install"')
    assert admission < preinst.index("if capsem_install_runs_inside_service /proc/self/cgroup")
    assert admission < preinst.index("capsem_retire_native_cohort \"$CAPSEM_DIR\"")
    assert "package is missing its candidate preinstall ledger resource" in preinst


def test_payload_is_embedded_after_strip_and_reverified_before_actual_control_budget() -> None:
    repack = (ROOT / "build_system/packaging/linux/repack-deb.sh").read_text()
    assert repack.index("strip_packaged_binaries\n") < repack.index('embed-preinstall-resource.py" embed')
    verify = repack.index('embed-preinstall-resource.py" verify')
    build = repack.index('dpkg-deb -b "$WORK_DIR/deb" "$WORK_DIR/candidate.deb"')
    budget = repack.index('dpkg-deb --ctrl-tarfile "$WORK_DIR/candidate.deb"')
    publish = repack.index('mv "$WORK_DIR/candidate.deb" "$OUTPUT_DEB"')
    assert verify < build < budget < publish
    assert "set -euo pipefail" in repack
