from __future__ import annotations

import importlib.util
import os
import subprocess
import sys
import time
from pathlib import Path

from rust_sources import production

PROJECT_ROOT = Path(__file__).resolve().parents[3]
PROOF_SCRIPT = (
    PROJECT_ROOT / "build_system" / "tests" / "helpers" / "prove_installed_shell.py"
)
CLI_CLIENT = PROJECT_ROOT / "crates" / "capsem" / "src" / "client.rs"


def _proof_module():
    spec = importlib.util.spec_from_file_location("prove_installed_shell", PROOF_SCRIPT)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def test_guest_command_hides_marker_and_writes_shared_home_proof() -> None:
    module = _proof_module()
    marker = "CAPSEM_GUEST_EXECUTION_REQUIRED"
    command = module.guest_marker_command(marker, ".proof-file")

    assert marker.encode() not in command
    assert b'tee "$HOME/.proof-file"' in command


def test_session_boot_failure_reads_the_reason_the_service_already_recorded() -> None:
    module = _proof_module()

    # A booting VM and a resumable stopped one still owe us a prompt.
    assert module.session_boot_failure({"status": "Running", "can_resume": False}) is None
    assert module.session_boot_failure({"status": "Stopped", "can_resume": True}) is None
    assert module.session_boot_failure({"profile_id": "code"}) is None

    assert module.session_boot_failure(
        {
            "status": "Defunct",
            "can_resume": False,
            "last_error": "failed to build VmConfig: rootfs hash mismatch\n",
        }
    ) == "session is Defunct: failed to build VmConfig: rootfs hash mismatch"

    assert module.session_boot_failure(
        {
            "status": "Stopped",
            "can_resume": False,
            "resume_blocked_reason": "kernel pin blake3:abc does not match abc",
        }
    ) == "session is Stopped: kernel pin blake3:abc does not match abc"

    silent = module.session_boot_failure({"status": "Incompatible", "can_resume": False})
    assert silent is not None and "Incompatible" in silent


def test_fail_fast_reads_the_fields_capsem_info_actually_serializes() -> None:
    """Guard the one contract that makes the fast-fail work.

    `capsem info --json` prints `SessionInfo` verbatim, so the proof's
    detection is only as good as those field names and lifecycle spellings.
    Renaming one in Rust would leave the proof silently waiting out its whole
    timeout again -- the exact failure this detection exists to end.
    """
    client = production(CLI_CLIENT)

    for field in ("pub status:", "pub can_resume:", "pub last_error:", "pub resume_blocked_reason:"):
        assert field in client, f"SessionInfo no longer serializes {field}"
    assert "pub enum VmLifecycleState {" in client
    for variant in ("Running", "Defunct", "Incompatible"):
        assert f"    {variant},\n" in client, f"VmLifecycleState no longer spells {variant}"

    module = _proof_module()
    assert {"Running", "Stopped", "Suspended", "Defunct", "Incompatible"} >= module.FATAL_SESSION_STATUSES


def _fake_capsem_with_dead_session(tmp_path: Path) -> tuple[Path, Path]:
    """A capsem whose session dies after `create` returns, as a boot crash does."""
    log = tmp_path / "calls.log"
    binary = tmp_path / "capsem"
    info_json = (
        '{"profile_id":"code","status":"Defunct","can_resume":false,'
        '"last_error":"failed to build VmConfig: rootfs hash mismatch"}'
    )
    binary.write_text(
        "#!/bin/sh\n"
        'printf \'%s\\n\' "$*" >> "$CAPSEM_FAKE_LOG"\n'
        'case "$1" in\n'
        "  create) exit 0 ;;\n"
        "  delete) exit 0 ;;\n"
        f"  info) printf '%s\\n' '{info_json}'; exit 0 ;;\n"
        # The TUI parks on its non-resumable screen: no prompt, ever.
        "  shell) sleep 120 ;;\n"
        "  *) exit 2 ;;\n"
        "esac\n",
        encoding="utf-8",
    )
    binary.chmod(0o755)
    return binary, log


def test_shell_proof_fails_fast_with_the_boot_error_of_an_unreachable_session(
    tmp_path: Path,
) -> None:
    binary, log = _fake_capsem_with_dead_session(tmp_path)
    env = os.environ.copy()
    env["CAPSEM_FAKE_LOG"] = str(log)
    env["HOME"] = str(tmp_path)

    started = time.monotonic()
    result = subprocess.run(
        [
            "python3",
            str(PROOF_SCRIPT),
            "--capsem",
            str(binary),
            "--marker",
            "CAPSEM_NEVER_REACHED",
            "--session-name",
            "dead-proof",
            "--profile",
            "code",
            "--startup-delay",
            "0",
            "--timeout",
            "120",
        ],
        cwd=PROJECT_ROOT,
        env=env,
        capture_output=True,
        text=True,
        timeout=60,
    )
    elapsed = time.monotonic() - started

    assert result.returncode != 0
    assert "failed to build VmConfig: rootfs hash mismatch" in result.stderr
    assert "session is Defunct" in result.stderr
    # The whole point: no 120s wait for a prompt that can never arrive.
    assert elapsed < 30, f"proof took {elapsed:.1f}s to report a dead session"
    # The session dir is the only copy of process.log; the caller's failure
    # evidence copy runs after this process exits.
    assert "delete dead-proof" not in log.read_text(encoding="utf-8").splitlines()


def _fake_capsem(tmp_path: Path, *, execute_input: bool) -> tuple[Path, Path]:
    log = tmp_path / "calls.log"
    binary = tmp_path / "capsem"
    shell_body = (
        'IFS= read -r command\n/bin/sh -c "$command"\nIFS= read -r _ || true\n'
        if execute_input
        else 'IFS= read -r command\nprintf "%s\\n" "$command"\nsleep 5\n'
    )
    binary.write_text(
        "#!/bin/sh\n"
        'printf \'%s\\n\' "$*" >> "$CAPSEM_FAKE_LOG"\n'
        'case "$1" in\n'
        "  create) exit 0 ;;\n"
        "  delete) exit 0 ;;\n"
        "  info) printf '{\"profile_id\":\"co-work\"}\\n'; exit 0 ;;\n"
        "  shell)\n"
        "    printf 'root@%s:~# ' \"$3\"\n"
        f"{shell_body}"
        "    ;;\n"
        "  *) exit 2 ;;\n"
        "esac\n",
        encoding="utf-8",
    )
    binary.chmod(0o755)
    return binary, log


def test_shell_proof_requires_guest_executed_marker(tmp_path: Path) -> None:
    binary, log = _fake_capsem(tmp_path, execute_input=True)
    env = os.environ.copy()
    env["CAPSEM_FAKE_LOG"] = str(log)
    env["HOME"] = str(tmp_path)

    result = subprocess.run(
        [
            "python3",
            str(PROOF_SCRIPT),
            "--capsem",
            str(binary),
            "--marker",
            "CAPSEM_TEST_GUEST_SHELL_OK",
            "--session-name",
            "proof-session",
            "--startup-delay",
            "0",
            "--timeout",
            "5",
        ],
        cwd=PROJECT_ROOT,
        env=env,
        capture_output=True,
        text=True,
        timeout=10,
    )

    assert result.returncode == 0, result.stderr
    assert "CAPSEM_TEST_GUEST_SHELL_OK" in result.stdout
    calls = log.read_text(encoding="utf-8").splitlines()
    assert calls[0] == "create --name proof-session"
    assert "shell --name proof-session" in calls
    assert calls[-1] == "delete proof-session"


def test_shell_proof_creates_session_with_requested_profile(tmp_path: Path) -> None:
    binary, log = _fake_capsem(tmp_path, execute_input=True)
    env = os.environ.copy()
    env["CAPSEM_FAKE_LOG"] = str(log)
    env["HOME"] = str(tmp_path)

    result = subprocess.run(
        [
            "python3",
            str(PROOF_SCRIPT),
            "--capsem",
            str(binary),
            "--marker",
            "CAPSEM_PROFILE_SHELL_OK",
            "--session-name",
            "profile-proof",
            "--profile",
            "co-work",
            "--startup-delay",
            "0",
            "--timeout",
            "5",
        ],
        cwd=PROJECT_ROOT,
        env=env,
        capture_output=True,
        text=True,
        timeout=10,
    )

    assert result.returncode == 0, result.stderr
    assert log.read_text(encoding="utf-8").splitlines()[0] == (
        "create --name profile-proof --profile co-work"
    )
    assert "info profile-proof --json" in log.read_text(encoding="utf-8").splitlines()


def test_shell_proof_rejects_typed_but_unexecuted_command(tmp_path: Path) -> None:
    binary, log = _fake_capsem(tmp_path, execute_input=False)
    env = os.environ.copy()
    env["CAPSEM_FAKE_LOG"] = str(log)
    env["HOME"] = str(tmp_path)

    result = subprocess.run(
        [
            "python3",
            str(PROOF_SCRIPT),
            "--capsem",
            str(binary),
            "--marker",
            "CAPSEM_MUST_NOT_MATCH_ECHOED_INPUT",
            "--session-name",
            "proof-session",
            "--startup-delay",
            "0",
            "--timeout",
            "1",
        ],
        cwd=PROJECT_ROOT,
        env=env,
        capture_output=True,
        text=True,
        timeout=10,
    )

    assert result.returncode != 0
    assert "guest shell marker was not observed" in result.stderr
    # A failed proof keeps its session: deleting it would remove the
    # process.log and serial.log that say why the guest never came up.
    assert "preserving session proof-session" in result.stderr
    assert "delete proof-session" not in log.read_text(encoding="utf-8").splitlines()


def test_shell_proof_waits_for_guest_prompt_and_sends_one_command(
    tmp_path: Path,
) -> None:
    log = tmp_path / "calls.log"
    binary = tmp_path / "capsem"
    binary.write_text(
        "#!/usr/bin/env python3\n"
        "import json\n"
        "import os\n"
        "import select\n"
        "import subprocess\n"
        "import sys\n"
        "import time\n"
        "from pathlib import Path\n"
        "\n"
        "log = Path(os.environ['CAPSEM_FAKE_LOG'])\n"
        "args = sys.argv[1:]\n"
        "with log.open('a', encoding='utf-8') as handle:\n"
        "    handle.write(' '.join(args) + '\\n')\n"
        "if args[0] in {'create', 'delete'}:\n"
        "    raise SystemExit(0)\n"
        "if args[0] == 'info':\n"
        "    print(json.dumps({'profile_id': 'co-work'}))\n"
        "    raise SystemExit(0)\n"
        "if args[0] != 'shell':\n"
        "    raise SystemExit(2)\n"
        "if select.select([sys.stdin], [], [], 0.25)[0]:\n"
        "    with log.open('a', encoding='utf-8') as handle:\n"
        "        handle.write('EARLY_INPUT\\n')\n"
        "    raise SystemExit(9)\n"
        "session = args[args.index('--name') + 1]\n"
        "print(f'root@{session}:~# ', end='', flush=True)\n"
        "command = sys.stdin.readline()\n"
        "with log.open('a', encoding='utf-8') as handle:\n"
        "    handle.write('COMMAND ' + command)\n"
        "subprocess.run(command, shell=True, check=True)\n"
        "time.sleep(0.25)\n",
        encoding="utf-8",
    )
    binary.chmod(0o755)
    env = os.environ.copy()
    env["CAPSEM_FAKE_LOG"] = str(log)
    env["HOME"] = str(tmp_path)

    result = subprocess.run(
        [
            "python3",
            str(PROOF_SCRIPT),
            "--capsem",
            str(binary),
            "--marker",
            "CAPSEM_READY_GUEST_SHELL_OK",
            "--session-name",
            "ready-proof",
            "--startup-delay",
            "0",
            "--timeout",
            "3",
        ],
        cwd=PROJECT_ROOT,
        env=env,
        capture_output=True,
        text=True,
        timeout=10,
    )

    assert result.returncode == 0, result.stderr
    calls = log.read_text(encoding="utf-8").splitlines()
    assert "EARLY_INPUT" not in calls
    assert len([line for line in calls if line.startswith("COMMAND ")]) == 1
