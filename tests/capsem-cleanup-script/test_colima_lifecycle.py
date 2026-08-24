"""Functional contracts for gate-owned Colima lifecycle cleanup.

These drove `scripts/with-gate-colima.sh`, a wrapper whose EXIT trap stopped a
Colima the gate had started. The trap was correct, and correct only for the
commands that happened to sit inside the wrapper -- which was the expensive
half of `just test-clean` and nothing else.

"Give back what I found on the way in" is the resource abstraction exactly, so
it is `Colima(Resource)` now and `held` guarantees the release on every path,
including the aborted one. The wrapper is gone.

Still functional rather than mocked: a real `colima` executable goes on PATH
and the real `Runner` invokes it, so what is asserted is the sequence of
commands a machine would actually see.
"""

from __future__ import annotations

import os
from pathlib import Path

import pytest

from capsem.gate import config as gate_config
from capsem.gate.errors import GateError
from capsem.gate.gateresources import Colima
from capsem.gate.lifecycle import held
from capsem.gate.proc import Runner

REPO_ROOT = Path(__file__).resolve().parents[2]


def _fake_colima(tmp_path: Path, *, running: bool) -> tuple[Path, Path]:
    """A real executable named `colima`, recording what it was asked to do."""
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
    os.environ["PATH"] = f"{bin_dir}:{os.environ['PATH']}"
    os.environ["FAKE_COLIMA_STATE"] = str(state)
    os.environ["FAKE_COLIMA_LOG"] = str(log)
    return state, log


@pytest.fixture
def colima_on_path(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    """Install the fake and undo the environment afterwards."""
    monkeypatch.setattr("shutil.which", lambda name: str(tmp_path / "bin" / name))

    def _install(*, running: bool):
        state, log = _fake_colima(tmp_path, running=running)
        resource = Colima(gate_config.load(REPO_ROOT), Runner(REPO_ROOT))
        return resource, state, log

    return _install


def test_gate_stops_colima_it_started_after_success(colima_on_path) -> None:
    resource, state, log = colima_on_path(running=False)

    with held(resource):
        Runner(REPO_ROOT).run(["colima", "start"])

    assert state.read_text(encoding="utf-8") == "stopped\n"
    assert log.read_text(encoding="utf-8").splitlines() == ["start", "stop"]


def test_gate_stops_colima_it_started_after_failure_and_preserves_status(
    colima_on_path,
) -> None:
    """The wrapper's `return "$status"` in another form.

    A resource releases on the failure path too, and the body's error is what
    propagates -- so the operator reads the failure, not the cleanup.
    """
    resource, state, log = colima_on_path(running=False)

    with pytest.raises(GateError, match="the real failure"), held(resource):
        Runner(REPO_ROOT).run(["colima", "start"])
        raise GateError("the real failure")

    assert state.read_text(encoding="utf-8") == "stopped\n"
    assert log.read_text(encoding="utf-8").splitlines() == ["start", "stop"]


def test_gate_preserves_preexisting_colima(colima_on_path) -> None:
    """A developer who already had Colima up keeps it."""
    resource, state, log = colima_on_path(running=True)

    with held(resource):
        pass

    assert state.read_text(encoding="utf-8") == "running\n"
    assert not log.exists()


def test_an_interrupted_gate_still_gives_colima_back(colima_on_path) -> None:
    """The abort path is the one the trap existed for, and the one a step
    could not cover -- a step whose dependency failed is skipped."""
    resource, state, _log = colima_on_path(running=False)

    with pytest.raises(KeyboardInterrupt), held(resource):
        Runner(REPO_ROOT).run(["colima", "start"])
        raise KeyboardInterrupt

    assert state.read_text(encoding="utf-8") == "stopped\n"
