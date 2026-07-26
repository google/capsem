from __future__ import annotations

import importlib.util
import os
import subprocess
import sys
from pathlib import Path


PROJECT_ROOT = Path(__file__).parent.parent
PROOF_SCRIPT = PROJECT_ROOT / "scripts" / "prove-installed-shell.py"


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
    assert log.read_text(encoding="utf-8").splitlines()[-1] == "delete proof-session"


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
