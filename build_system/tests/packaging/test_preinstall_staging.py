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
    restriction: str | None = None,
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
capsem_admit_preinstall_ledger {q(target)} {crypto.sha256(expected).hexdigest()} {len(expected)} 15 2 {q(restriction) if restriction is not None else ""}
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
        "fixture-user", "-c", 'exec "$@"',
        "capsem-preinstall", str(tmp_path / "stage/capsem-ledger"),
        "--preinstall-ledger-admission", "--target-user", "fixture-user",
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


def package_admission_flow(*, platform: str, fail_at: int = 0, preserved: bool = False, legacy: bool = False, manager_status: int = 0, systemd_booted: bool = True, systemctl_available: bool = True) -> subprocess.CompletedProcess[str]:
    """Execute original maintainer-script control flow with labelled OS effects."""
    if platform == "Darwin":
        source = (ROOT / "build_system/packaging/macos/pkg-scripts/preinstall").read_text()
        start = 'CAPSEM_INSTALL_PHASE="admit_candidate_ledger"'
    else:
        source = (ROOT / "build_system/packaging/linux/deb-preinst.sh").read_text()
        start = 'CAPSEM_INSTALL_PHASE="admit_existing_ledger"'
    body = source[source.index(start):]
    setup = f'''
set -euo pipefail
USER=fixture-user
TARGET_USER=fixture-user
TARGET_UID=12345
USER_HOME=/nonexistent-capsem-source-fixture
CAPSEM_DIR=/nonexistent-capsem-source-fixture/.capsem
calls=0
# Label only the platform metadata/tool-presence inputs, not the script branch.
function [ {{
    if builtin [ "$#" -eq 3 ] && builtin [ "$1" = -d ] && builtin [ "$2" = /run/systemd/system ]; then return {0 if systemd_booted else 1}; fi
    builtin [ "$@"
}}
command() {{
    if builtin [ "$#" -eq 2 ] && builtin [ "$1" = -v ] && builtin [ "$2" = systemctl ]; then return {0 if systemctl_available else 1}; fi
    builtin command "$@"
}}
capsem_run_preinstall_ledger_admission() {{
    calls=$((calls + 1))
    printf 'admit:%s:%s\\n' "$calls" "$*"
    if [ "$calls" -eq {fail_at} ]; then return 23; fi
    if [ {int(legacy)} -eq 1 ] && [ "${{2:-}}" = --require-current-host ]; then return 24; fi
}}
capsem_install_runs_inside_service() {{ return {0 if preserved else 1}; }}
capsem_stop_launch_agent() {{ echo manager-stopped; return {manager_status}; }}
capsem_stop_systemd_service() {{ echo manager-stopped; return {manager_status}; }}
capsem_retire_native_cohort() {{ echo retire; }}
id() {{ echo 12345; }}
su() {{ echo remove-user; }}
rm() {{ echo remove-package; }}
'''
    return subprocess.run(["bash", "-c", setup + body], capture_output=True, text=True, timeout=10)


@pytest.mark.parametrize("platform", ["Darwin", "Linux"])
@pytest.mark.parametrize("fail_at", [1, 2])
def test_original_package_flow_rechecks_after_drain_before_removal(platform: str, fail_at: int) -> None:
    result = package_admission_flow(platform=platform, fail_at=fail_at)
    assert result.returncode == 23, result.stdout + result.stderr
    lines = result.stdout.splitlines()
    assert not any(line.startswith("remove-") for line in lines)
    admissions = [line for line in lines if line.startswith("admit:")]
    assert len(admissions) == fail_at
    assert ("retire" in lines) == (fail_at == 2)
    if fail_at == 2:
        assert lines.index("retire") < lines.index(admissions[1])


def test_linux_in_service_legacy_refuses_before_unpack_but_current_host_is_preserved() -> None:
    refused = package_admission_flow(platform="Linux", preserved=True, legacy=True)
    assert refused.returncode == 24, refused.stdout + refused.stderr
    assert "--require-current-host" in refused.stdout
    assert "retire" not in refused.stdout and "event=complete" not in refused.stdout
    current = package_admission_flow(platform="Linux", preserved=True)
    assert current.returncode == 0, current.stderr
    assert "--require-current-host" in current.stdout and "event=complete" in current.stdout
    assert "retire" not in current.stdout


@pytest.mark.parametrize("platform", ["Darwin", "Linux"])
def test_only_current_host_restriction_reaches_exact_candidate_argv(tmp_path: Path, platform: str) -> None:
    result = invoke(tmp_path, platform=platform, restriction="--require-current-host")
    assert result.returncode == 0, result.stderr
    argv = tmp_path / ("su-argv" if platform == "Darwin" else "runuser-argv")
    assert argv.read_text().splitlines()[-1] == "--require-current-host"
    assert not (tmp_path / "stage").exists()


@pytest.mark.parametrize("restriction", ["--database=/foreign", "--require-current-host=no", ""])
def test_unknown_restriction_never_stages_or_executes(tmp_path: Path, restriction: str) -> None:
    result = invoke(tmp_path, restriction=restriction)
    assert result.returncode != 0
    assert not (tmp_path / "stage").exists()
    assert not (tmp_path / "runuser-argv").exists()


@pytest.mark.parametrize("platform", ["Darwin", "Linux"])
def test_unverified_manager_drain_prevents_retirement_and_payload_removal(platform: str) -> None:
    result = package_admission_flow(platform=platform, manager_status=31)
    assert result.returncode == 31, result.stderr
    lines = result.stdout.splitlines()
    assert lines.count("manager-stopped") == 1
    assert "retire" not in lines
    assert not any(line.startswith("remove-") for line in lines)
    assert not any(line.startswith("admit:2:") for line in lines)


@pytest.mark.parametrize("systemctl_available", [False, True])
def test_non_systemd_installs_still_drain_and_recheck_before_unpack(systemctl_available: bool) -> None:
    result = package_admission_flow(platform="Linux", systemd_booted=False, systemctl_available=systemctl_available, manager_status=31)
    assert result.returncode == 0, result.stdout + result.stderr
    lines = result.stdout.splitlines()
    assert "manager-stopped" not in lines
    ordered = [line for line in lines if line.startswith("admit:") or line == "retire"]
    assert ordered == ["admit:1:fixture-user", "retire", "admit:2:fixture-user"]
    assert "event=complete" in result.stdout


@pytest.mark.parametrize("systemctl_available", [False, True])
def test_booted_systemd_cannot_skip_unavailable_user_manager(systemctl_available: bool) -> None:
    result = package_admission_flow(platform="Linux", systemd_booted=True, systemctl_available=systemctl_available, manager_status=31)
    assert result.returncode == 31, result.stdout + result.stderr
    assert "manager-stopped" in result.stdout
    assert "retire" not in result.stdout and "admit:2:" not in result.stdout
    assert "event=complete" not in result.stdout
