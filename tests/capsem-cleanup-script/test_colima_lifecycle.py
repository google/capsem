"""Functional contracts for gate-owned Colima lifecycle cleanup."""

from __future__ import annotations

import os
import subprocess
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
WRAPPER = REPO_ROOT / "scripts" / "with-gate-colima.sh"


def _fake_colima(tmp_path: Path, *, running: bool) -> tuple[dict[str, str], Path, Path]:
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    state = tmp_path / "state"
    log = tmp_path / "calls.log"
    state.write_text("running\n" if running else "stopped\n", encoding="utf-8")
    colima = bin_dir / "colima"
    colima.write_text(
        """#!/bin/bash
set -euo pipefail
case "${1:-}" in
    status)
        grep -qx running "$FAKE_COLIMA_STATE"
        ;;
    start)
        printf 'start\\n' >> "$FAKE_COLIMA_LOG"
        printf 'running\\n' > "$FAKE_COLIMA_STATE"
        ;;
    stop)
        printf 'stop\\n' >> "$FAKE_COLIMA_LOG"
        printf 'stopped\\n' > "$FAKE_COLIMA_STATE"
        ;;
    *)
        exit 2
        ;;
esac
""",
        encoding="utf-8",
    )
    colima.chmod(0o755)
    env = os.environ.copy()
    env.update(
        {
            "PATH": f"{bin_dir}:{env['PATH']}",
            "FAKE_COLIMA_STATE": str(state),
            "FAKE_COLIMA_LOG": str(log),
        }
    )
    return env, state, log


def _run(tmp_path: Path, command: str, *, running: bool) -> tuple[subprocess.CompletedProcess[str], Path, Path]:
    env, state, log = _fake_colima(tmp_path, running=running)
    result = subprocess.run(
        [str(WRAPPER), "bash", "-c", command],
        cwd=REPO_ROOT,
        env=env,
        text=True,
        capture_output=True,
        check=False,
    )
    return result, state, log


def test_gate_stops_colima_it_started_after_success(tmp_path: Path) -> None:
    result, state, log = _run(tmp_path, "colima start", running=False)

    assert result.returncode == 0, result.stderr
    assert state.read_text(encoding="utf-8") == "stopped\n"
    assert log.read_text(encoding="utf-8").splitlines() == ["start", "stop"]


def test_gate_stops_colima_it_started_after_failure_and_preserves_status(tmp_path: Path) -> None:
    result, state, log = _run(tmp_path, "colima start; exit 7", running=False)

    assert result.returncode == 7
    assert state.read_text(encoding="utf-8") == "stopped\n"
    assert log.read_text(encoding="utf-8").splitlines() == ["start", "stop"]


def test_gate_preserves_preexisting_colima(tmp_path: Path) -> None:
    result, state, log = _run(tmp_path, "true", running=True)

    assert result.returncode == 0, result.stderr
    assert state.read_text(encoding="utf-8") == "running\n"
    assert not log.exists()
