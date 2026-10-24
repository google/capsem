"""Original installer control owner; labelled manager commands, no service mutation."""

import shlex
import subprocess
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[3]
RETIRE = ROOT / "build_system/packaging/shared/retire-cohort"


def probe(platform: str, status: int, output: str, *, stop: int = 1, diagnostic: str = "") -> subprocess.CompletedProcess[str]:
    if platform == "Darwin":
        command = f'''
function /bin/launchctl {{
    if [ "$1" = print ]; then
        [ "${{LC_ALL:-}}" = C ] || return 99
        printf '%s' {shlex.quote(output)} >&2
        return {status}
    fi
    return {stop}
}}
capsem_stop_launch_agent gui/12345 /absent-source-plist
'''
    else:
        command = f'''
su() {{
    case "$3" in
        *'systemctl --user stop'*) return {stop} ;;
        *'systemctl --user show'*) printf '%s' {shlex.quote(output)}; printf '%s' {shlex.quote(diagnostic)} >&2; return {status} ;;
        *) return 99 ;;
    esac
}}
capsem_stop_systemd_service fixture-user /run/user/12345
'''
    return subprocess.run(["bash", "-c", 'set -euo pipefail\nsource "$1"\n' + command, "source-fixture", str(RETIRE)], capture_output=True, text=True, timeout=10)


def absent(label: str = "com.capsem.service", domain: str = "gui: 12345") -> str:
    return f'Bad request.\nCould not find service "{label}" in domain for user {domain}'


@pytest.mark.parametrize("stop", [0, 1])
def test_mac_exact_target_absence_accepts_already_stopped_or_successful_unload(stop: int) -> None:
    result = probe("Darwin", 113, absent(), stop=stop)
    assert result.returncode == 0, result.stderr


@pytest.mark.parametrize("status,output", [
    (0, "still loaded"), (1, "permission denied"), (127, "missing tool"),
    (113, absent("foreign.service")), (113, absent(domain="gui: 54321")),
    (113, absent() + "\nother output"), (1, absent()),
])
def test_mac_query_failure_or_wrong_target_cannot_certify_drain(status: int, output: str) -> None:
    assert probe("Darwin", status, output, stop=0).returncode != 0


@pytest.mark.parametrize("load", ["loaded", "not-found", "masked"])
def test_linux_exact_inactive_dead_state_accepts_stopped_unit(load: str) -> None:
    assert probe("Linux", 0, f"SubState=dead\nLoadState={load}\nActiveState=inactive\n").returncode == 0


@pytest.mark.parametrize("status,output", [
    (1, "LoadState=not-found\nActiveState=inactive\nSubState=dead"),
    (0, "LoadState=loaded\nActiveState=active\nSubState=running"),
    (0, "LoadState=loaded\nActiveState=failed\nSubState=failed"),
    (0, "ActiveState=inactive"),
    (0, "LoadState=\nLoadState=loaded\nActiveState=inactive\nSubState=dead"),
    (0, "LoadState=loaded\nActiveState=inactive\nSubState=dead\nUnknown=value"),
])
def test_linux_unverifiable_or_still_active_manager_state_refuses(status: int, output: str) -> None:
    assert probe("Linux", status, output).returncode != 0


def test_manager_failure_does_not_disclose_captured_service_details() -> None:
    canary = "synthetic-private-manager-detail"
    for platform in ["Darwin", "Linux"]:
        result = probe(platform, 1, canary)
        assert result.returncode != 0
        assert canary not in result.stdout + result.stderr


def test_linux_failed_query_stderr_is_not_copied_to_install_log() -> None:
    canary = "synthetic-private-stderr-query-canary"
    result = probe("Linux", 1, "", diagnostic=canary)
    assert result.returncode != 0
    assert canary not in result.stdout + result.stderr
    assert "cannot verify selected systemd service state" in result.stderr
