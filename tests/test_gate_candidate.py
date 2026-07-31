"""The three guarantees `just test` makes on the way out.

These were text assertions against the recipe when the gate was shell: the
trap had to be armed, had to `return "$status"` rather than `exit "$status"`,
and must not be disarmed early. Each was checked by grepping the justfile,
which proves the code was written a particular way and not that it behaves a
particular way.

`try`/`finally` removes the trap-status hazard entirely -- there is no `$?` to
misread -- so these assert the behaviour instead: an interrupted run reports
the interrupt, a leaked process fails an otherwise-passing run, and a failing
run keeps its own error rather than the cleanup's.
"""

from __future__ import annotations

from pathlib import Path

import pytest
from helpers.gate import RecordingRunner

from capsem.gate import config as gate_config
from capsem.gate.candidate import CandidateGate, keep_awake
from capsem.gate.errors import GateError

PROJECT_ROOT = Path(__file__).resolve().parents[1]
CONFIG = gate_config.load(PROJECT_ROOT)
HEAD = "abcdef1234567890"
DIGEST = "sha256:cafe"


def _checkout(tmp_path: Path) -> Path:
    (tmp_path / "config").mkdir(parents=True)
    (tmp_path / "config" / "gate.toml").write_text(
        (PROJECT_ROOT / "config" / "gate.toml").read_text(encoding="utf-8")
    )
    return tmp_path


class Running(RecordingRunner):
    """A gate whose source state holds still unless a test moves it."""

    head = HEAD
    digest = DIGEST

    def execute(self, command):
        rendered = str(command)
        if "rev-parse HEAD" in rendered:
            return self._answer(command, self.head)
        if "source-state-digest" in rendered:
            return self._answer(command, self.digest)
        return super().execute(command)

    def _answer(self, command, value: str):
        completed = super().execute(command)
        completed.stdout = value
        return completed


def _gate(tmp_path: Path, **kwargs) -> tuple[CandidateGate, Running]:
    runner = Running(_checkout(tmp_path), **kwargs)
    return CandidateGate(runner), runner


# ---------------------------------------------------------------------------
# The source state under test
# ---------------------------------------------------------------------------


def test_the_source_state_is_captured_before_anything_runs(tmp_path: Path) -> None:
    gate, runner = _gate(tmp_path)

    gate.run()

    runner.assert_order(
        r"rev-parse HEAD", r"source-state-digest\.py", r"just _test-fast"
    )


def test_the_process_baseline_precedes_anything_that_can_spawn_one(
    tmp_path: Path,
) -> None:
    """Taken later, it would absorb this run's own processes -- and a developer's
    dev daemon would be blamed for a leak it did not cause."""
    gate, runner = _gate(tmp_path)

    gate.run()

    runner.assert_order(
        r"check-orphan-processes\.py baseline", r"just _test-fast"
    )


def test_the_fast_module_runs_before_the_expensive_one(tmp_path: Path) -> None:
    """Its failures come back in minutes rather than after the Docker and VM
    work."""
    gate, runner = _gate(tmp_path)

    gate.run()

    runner.assert_order(r"just _test-fast", r"with-gate-colima\.sh just _test-candidate")


def test_a_head_that_moved_mid_run_fails(tmp_path: Path) -> None:
    """A gate that qualified a HEAD nobody has proved nothing about anything."""
    gate, runner = _gate(tmp_path)
    original = Running.execute

    def moving(self, command):
        if "rev-parse HEAD" in str(command) and runner.ran("just _test-fast"):
            self.head = "0000000000000000"
        return original(self, command)

    Running.execute = moving
    try:
        with pytest.raises(GateError, match="source HEAD changed"):
            gate.run()
    finally:
        Running.execute = original
        Running.head = HEAD


def test_a_working_tree_the_gate_edited_fails(tmp_path: Path) -> None:
    gate, runner = _gate(tmp_path)
    original = Running.execute

    def dirtying(self, command):
        if "source-state-digest" in str(command) and runner.ran("just _test-fast"):
            self.digest = "sha256:changed"
        return original(self, command)

    Running.execute = dirtying
    try:
        with pytest.raises(GateError, match="changed the source working tree"):
            gate.run()
    finally:
        Running.execute = original
        Running.digest = DIGEST


# ---------------------------------------------------------------------------
# Closing out
# ---------------------------------------------------------------------------


def test_a_leaked_process_fails_an_otherwise_passing_run(tmp_path: Path) -> None:
    """The whole point: a run that did everything right and left a service
    behind is not a run that passed."""
    gate, _ = _gate(tmp_path, failures=["orphan-processes.py check"])

    with pytest.raises(GateError, match="outlived the gate"):
        gate.run()


def test_the_count_still_happens_when_the_gate_fails(tmp_path: Path) -> None:
    """An aborted run is the one that skips its own cleanup, so it is exactly
    the run whose survivors need counting."""
    gate, runner = _gate(tmp_path, failures=["just _test-fast"])

    with pytest.raises(GateError):
        gate.run()

    assert runner.ran(r"check-orphan-processes\.py check")


def test_a_failing_run_keeps_its_own_error_rather_than_the_cleanups(
    tmp_path: Path,
) -> None:
    """Otherwise the operator reads about a leaked process and never sees the
    failure that caused it."""
    gate, _ = _gate(
        tmp_path, failures=["just _test-fast", "orphan-processes.py check"]
    )

    with pytest.raises(GateError) as failure:
        gate.run()

    assert "_test-fast" in str(failure.value)
    assert "outlived" not in str(failure.value)


def test_an_interrupted_run_is_never_reported_as_a_pass(tmp_path: Path) -> None:
    """The hazard `try`/`finally` removes.

    Inside an EXIT trap `$?` is the last command's status, which on Ctrl-C is
    0, so `exit "$status"` discarded the shell's own 130 and turned an abort
    into a green gate.
    """
    gate, runner = _gate(tmp_path)
    original = Running.execute

    def interrupting(self, command):
        if "just _test-fast" in str(command):
            raise KeyboardInterrupt
        return original(self, command)

    Running.execute = interrupting
    try:
        with pytest.raises(KeyboardInterrupt):
            gate.run()
    finally:
        Running.execute = original

    assert runner.ran(r"check-orphan-processes\.py check"), (
        "the count must happen on the abort path too"
    )


def test_failure_evidence_is_captured_and_labelled_with_the_head(
    tmp_path: Path,
) -> None:
    gate, runner = _gate(tmp_path, failures=["just _test-fast"])

    with pytest.raises(GateError):
        gate.run()

    captured = runner.matching(r"capture-failure")
    assert captured
    assert HEAD[:12] in captured[0]


def test_a_passing_run_captures_no_failure_evidence(tmp_path: Path) -> None:
    gate, runner = _gate(tmp_path)

    gate.run()

    assert not runner.ran(r"capture-failure")


# ---------------------------------------------------------------------------
# Staying awake
# ---------------------------------------------------------------------------


def test_macos_wraps_the_gate_so_the_machine_cannot_sleep_through_it(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Darwin")
    monkeypatch.setattr("shutil.which", lambda _name: "/usr/bin/caffeinate")
    monkeypatch.delenv(CONFIG.candidate.keep_awake_marker, raising=False)

    prefix = keep_awake(Running(_checkout(tmp_path)))

    assert prefix is not None
    assert prefix[0] == "caffeinate"
    assert f"{CONFIG.candidate.keep_awake_marker}=1" in prefix


def test_the_wrapper_is_applied_once(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    """Without the marker it would re-exec itself forever."""
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Darwin")
    monkeypatch.setenv(CONFIG.candidate.keep_awake_marker, "1")

    assert keep_awake(Running(_checkout(tmp_path))) is None


def test_linux_needs_no_wrapper(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Linux")

    assert keep_awake(Running(_checkout(tmp_path))) is None


def test_a_macos_host_without_caffeinate_is_told_why_it_matters(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Darwin")
    monkeypatch.setattr("shutil.which", lambda _name: None)
    monkeypatch.delenv(CONFIG.candidate.keep_awake_marker, raising=False)

    with pytest.raises(GateError, match="unattended"):
        keep_awake(Running(_checkout(tmp_path)))
