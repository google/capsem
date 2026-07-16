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
        "  shell)\n"
        "    printf 'guest shell ready\\n'\n"
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
