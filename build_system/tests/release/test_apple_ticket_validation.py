"""Exercise the actual validation entrypoint with a controlled xcrun process."""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

import pytest
from capsem_builder.gate.releaseschema import AppleTicketValidationConfig
from capsem_builder.release.tools.validate_apple_ticket import validate

LOST_CONNECTION = (
    'Error Domain=NSURLErrorDomain Code=-1005 "The network connection was lost." '
    "https://api.apple-cloudkit.com/database/1/com.apple.gk.ticket-delivery/"
    "production/public/records/lookup"
)


def _xcrun(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    outcomes: list[tuple[int | str, str]],
) -> Path:
    state = tmp_path / "attempts"
    executable = tmp_path / "xcrun"
    executable.write_text(
        f"#!{sys.executable}\n"
        "import sys,time\nfrom pathlib import Path\n"
        f"state=Path({str(state)!r})\noutcomes={outcomes!r}\n"
        "assert sys.argv[1:3] == ['stapler','validate']\n"
        "assert sys.argv[3].endswith('candidate package.pkg')\n"
        "attempt=int(state.read_text()) if state.exists() else 0\n"
        "state.write_text(str(attempt+1))\n"
        "code,message=outcomes[min(attempt,len(outcomes)-1)]\n"
        "if code == 'timeout': time.sleep(60)\n"
        "print(message,file=sys.stderr,flush=True)\nsys.exit(code)\n"
    )
    executable.chmod(0o755)
    monkeypatch.setenv("PATH", f"{tmp_path}{os.pathsep}{os.environ['PATH']}")
    return state


@pytest.mark.parametrize(
    ("outcomes", "expected", "attempts"),
    [
        ([(0, "The validate action worked!")], 0, 1),
        ([(68, LOST_CONNECTION), (0, "The validate action worked!")], 0, 2),
        ([(68, "Ticket is invalid")], 68, 1),
        ([(68, "Could not find ticket. HTTP 404")], 68, 1),
        ([(77, LOST_CONNECTION)], 77, 1),
        ([(68, LOST_CONNECTION), (65, "Invalid signature")], 65, 2),
        ([(68, LOST_CONNECTION.replace("api.apple-cloudkit.com", "unrelated.invalid"))], 68, 1),
        ([(68, LOST_CONNECTION)], 68, 3),
        ([("timeout", "")], 124, 1),
    ],
)
def test_validation_is_mandatory_and_only_lost_cloudkit_connections_retry(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    outcomes: list[tuple[int | str, str]],
    expected: int,
    attempts: int,
) -> None:
    state = _xcrun(tmp_path, monkeypatch, outcomes)
    settings = AppleTicketValidationConfig(
        attempts=3,
        retry_delay_seconds=0,
        timeout_seconds=1 if outcomes[0][0] == "timeout" else 10,
    )
    assert validate(tmp_path / "candidate package.pkg", settings) == expected
    assert int(state.read_text()) == attempts


def test_workflow_module_entrypoint_requires_eventual_success(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    state = _xcrun(tmp_path, monkeypatch, [(68, LOST_CONNECTION), (0, "Validated")])
    result = subprocess.run(
        [
            sys.executable,
            "-m",
            "capsem_builder.release.tools.validate_apple_ticket",
            str(tmp_path / "candidate package.pkg"),
        ],
        cwd=Path(__file__).resolve().parents[3],
        stdin=subprocess.DEVNULL,
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )
    assert result.returncode == 0, result.stdout + result.stderr
    assert int(state.read_text()) == 2
    assert "retrying required ticket validation" in result.stderr
