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
    printf '%s' {shlex.quote(diagnostic)} >&2
    printf '%s' {shlex.quote(diagnostic)}
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


def test_failed_bootout_never_unloads_a_user_writable_foreign_label(tmp_path: Path) -> None:
    plist = tmp_path / "user-writable.plist"
    original = b'<plist><dict><key>Label</key><string>foreign.root.service</string></dict></plist>'
    plist.write_bytes(original)
    actions = tmp_path / "actions"
    script = f'''
set -euo pipefail
source {shlex.quote(str(RETIRE))}
function /bin/launchctl {{
    printf '%s\\n' "$*" >> {shlex.quote(str(actions))}
    if [ "$1" = print ]; then
        printf '%s' {shlex.quote(absent())} >&2
        return 113
    fi
    return 1
}}
capsem_stop_launch_agent gui/12345 {shlex.quote(str(plist))}
'''
    result = subprocess.run(["bash", "-c", script], text=True, capture_output=True, timeout=10)
    assert result.returncode == 0, result.stderr
    assert actions.read_text().splitlines() == [
        "bootout gui/12345/com.capsem.service", "print gui/12345/com.capsem.service",
    ]
    assert plist.read_bytes() == original


@pytest.mark.parametrize("stop", [0, 5, 127])
def test_mac_refusal_records_exact_bootout_exit_without_manager_output(stop: int) -> None:
    canary = "synthetic-private-bootout-detail"
    result = probe("Darwin", 0, "loaded-query-private-detail", stop=stop, diagnostic=canary)
    assert result.returncode == 1
    assert f"bootout_exit={stop} print_exit=0" in result.stderr
    assert canary not in result.stdout + result.stderr
    assert "loaded-query-private-detail" not in result.stdout + result.stderr


def test_mac_unverifiable_absence_retains_both_original_exit_codes() -> None:
    result = probe("Darwin", 77, "private-query-error", stop=5)
    assert result.returncode == 1
    assert "bootout_exit=5 print_exit=77" in result.stderr
    assert "private-query-error" not in result.stdout + result.stderr
