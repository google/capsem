"""The three guarantees `just test` makes on the way out.

These were text assertions against the recipe when the gate was shell: the trap
had to be armed, had to `return "$status"` rather than `exit "$status"`, and
must not be disarmed early. Each was checked by grepping the justfile, which
proves the code was written a particular way and not that it behaves a
particular way.

They then became assertions about an imperative `CandidateGate.run()`. The gate
is a composed plan now, and the guarantees split cleanly by *when* they must
hold:

  the source state is a pair of steps, because re-asserting it must not happen
  when the gate failed -- the failure is the report

  the process count and the failure evidence are `Resource`s, because they must
  happen on every path including the aborted one, and a step whose dependency
  failed is skipped

So the claims are unchanged and the evidence moved to whichever of those two
places now owns each one.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path

import pytest
from helpers.gate import RecordingRunner

from capsem.gate import cli  # noqa: F401 - imported so every command registers
from capsem.gate import config as gate_config
from capsem.gate.candidate import CandidateCommand, keep_awake
from capsem.gate.command import GateCommand
from capsem.gate.context import Context
from capsem.gate.errors import GateError
from capsem.gate.lifecycle import held
from capsem.gate.sourcestate import RecordSourceState, RequireSourceUnchanged

# The Colima lifecycle has its own home in
# tests/capsem-cleanup-script/test_colima_lifecycle.py, where it is driven
# against a real executable rather than a recording runner.

PROJECT_ROOT = Path(__file__).resolve().parents[1]


#: `resources()` takes the runner it should build with; these tests ask
#: *what* is held, so any runner will do.
def _resource_runner():
    from helpers.gate import RecordingRunner

    return RecordingRunner(PROJECT_ROOT)


RUNNER_FOR_RESOURCES = _resource_runner()
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


def _command(root: Path, **kwargs) -> CandidateCommand:
    runner = Running(root, **kwargs)
    return GateCommand.registry["candidate"](
        runner, argparse.Namespace(dry_run=False, graph=False, timing=False)
    )


def _context(root: Path, **kwargs) -> Context:
    """A context for a gate that is *running*.

    The journal matters: recording the source state is something a run does,
    so a context with nothing recording behind it declines to (see
    `test_recording_the_source_state_needs_a_run_to_record_into`).
    """
    from helpers.gate import RecordingJournal

    return Context(Running(root, **kwargs), gate_config.for_root(root), journal=RecordingJournal())


def _plan():
    return _command(PROJECT_ROOT)._describe()


# ---------------------------------------------------------------------------
# The source state under test
# ---------------------------------------------------------------------------


def test_the_source_state_is_captured_before_anything_runs() -> None:
    labels = list(_plan().labels)

    assert labels[0] == "source.record"


def test_the_process_baseline_precedes_anything_that_can_spawn_one() -> None:
    """Taken later, it would absorb this run's own processes -- and a
    developer's dev daemon would be blamed for a leak it did not cause.

    Acquisition order is the guarantee now: resources are taken left to right
    and released in reverse, so the baseline is first taken and last compared.
    """
    names = [resource.name for resource in _command(PROJECT_ROOT).resources(RUNNER_FOR_RESOURCES)]

    assert names[0] == "orphan-accounting"


def test_the_fast_module_runs_before_the_expensive_one() -> None:
    """Its failures come back in minutes rather than after the Docker and VM
    work."""
    labels = list(_plan().labels)
    fast = next(i for i, label in enumerate(labels) if label.startswith("fast."))
    static = next(i for i, label in enumerate(labels) if label.startswith("static."))

    assert fast < static


def test_a_head_that_moved_mid_run_fails(tmp_path: Path) -> None:
    """A gate that qualified a HEAD nobody has proved nothing about anything."""
    root = _checkout(tmp_path)
    context = _context(root)
    RecordSourceState().perform(context)

    moved = _context(root)
    moved.runner.head = "0000000000000000"
    with pytest.raises(GateError, match="source HEAD changed"):
        RequireSourceUnchanged().perform(moved)


def test_a_working_tree_the_gate_edited_fails(tmp_path: Path) -> None:
    root = _checkout(tmp_path)
    RecordSourceState().perform(_context(root))

    dirtied = _context(root)
    dirtied.runner.digest = "sha256:changed"
    with pytest.raises(GateError, match="changed the source working tree"):
        RequireSourceUnchanged().perform(dirtied)


def test_a_verification_with_nothing_recorded_refuses(tmp_path: Path) -> None:
    """A missing record must not read as agreement."""
    with pytest.raises(GateError, match="never recorded"):
        RequireSourceUnchanged().perform(_context(_checkout(tmp_path)))


# ---------------------------------------------------------------------------
# Closing out: what must happen on every path
# ---------------------------------------------------------------------------


def _accounting(root: Path, **kwargs):
    from capsem.gate.candidate import OrphanAccounting

    runner = Running(root, **kwargs)
    return OrphanAccounting(gate_config.for_root(root), runner), runner


def test_a_leaked_process_fails_an_otherwise_passing_run(tmp_path: Path) -> None:
    """The whole point: a run that did everything right and left a service
    behind is not a run that passed."""
    accounting, _ = _accounting(_checkout(tmp_path), failures=["orphan-processes.py check"])
    accounting.acquire()

    with pytest.raises(GateError, match="outlived the gate"):
        accounting.release()


def test_the_count_still_happens_when_the_gate_fails(tmp_path: Path) -> None:
    """An aborted run is the one that skips its own cleanup, so it is exactly
    the run whose survivors need counting.

    A `Resource` releases on every path, which is why the accounting is one
    rather than a step -- a step whose dependency failed is skipped.
    """
    accounting, runner = _accounting(_checkout(tmp_path))

    with pytest.raises(GateError, match="boom"), held(accounting):
        raise GateError("boom")

    assert runner.ran(r"check-orphan-processes\.py check")


def test_a_failing_run_keeps_its_own_error_rather_than_the_cleanups(
    tmp_path: Path,
) -> None:
    """Otherwise the operator reads about a leaked process and never sees the
    failure that caused it."""
    accounting, _ = _accounting(_checkout(tmp_path), failures=["orphan-processes.py check"])

    with pytest.raises(GateError) as failure, held(accounting):
        raise GateError("the real failure")

    assert "the real failure" in str(failure.value)


def test_an_interrupted_run_is_never_reported_as_a_pass(tmp_path: Path) -> None:
    """The hazard `try`/`finally` removes.

    Inside an EXIT trap `$?` is the last command's status, which on Ctrl-C is
    0, so `exit "$status"` discarded the shell's own 130 and turned an abort
    into a green gate. An interrupt propagates through `held` unless something
    explicitly swallows it.
    """
    accounting, runner = _accounting(_checkout(tmp_path))

    with pytest.raises(KeyboardInterrupt), held(accounting):
        raise KeyboardInterrupt

    assert runner.ran(r"check-orphan-processes\.py check"), (
        "the count must happen on the abort path too"
    )


def test_failure_evidence_is_captured_and_labelled_with_the_head(
    tmp_path: Path,
) -> None:
    """`preserve` runs only on failure and before release, because release is
    what destroys the evidence."""
    from capsem.gate.candidate import FailureEvidence

    root = _checkout(tmp_path)
    RecordSourceState().perform(_context(root))
    runner = Running(root)
    evidence = FailureEvidence(gate_config.for_root(root), runner)

    with pytest.raises(GateError), held(evidence):
        raise GateError("boom")

    captured = runner.matching(r"capture-failure")
    assert captured
    assert HEAD[:12] in captured[0]


def test_a_passing_run_captures_no_failure_evidence(tmp_path: Path) -> None:
    from capsem.gate.candidate import FailureEvidence

    root = _checkout(tmp_path)
    runner = Running(root)

    with held(FailureEvidence(gate_config.for_root(root), runner)):
        pass

    assert not runner.ran(r"capture-failure")


def test_the_gate_holds_everything_that_must_be_given_back() -> None:
    """The set, so a later change cannot quietly drop one."""
    names = {resource.name for resource in _command(PROJECT_ROOT).resources(RUNNER_FOR_RESOURCES)}

    assert names == {"orphan-accounting", "workspace", "colima", "failure-evidence"}


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


# ---------------------------------------------------------------------------
# Observing a plan is not running one
# ---------------------------------------------------------------------------


def test_recording_the_source_state_needs_a_run_to_record_into(tmp_path: Path) -> None:
    """`tests/helpers/gate.py` runs the real candidate plan against a
    recording runner to read back the argv it would issue. That stubs
    subprocesses -- it does not stub filesystem actions, so this step wrote
    the gate's own `target/gate-source-state.json` with the recorder's empty
    output, and the gate's `source.verify` then reported

        source HEAD changed while the gate was running:  -> <head>

    for a tree nobody had touched. It only reached that far once the last
    step in the plan started passing.

    A run's identity belongs to a run. With no journal recording one there is
    nothing to identify, and writing anyway is how an observer corrupts the
    thing it is observing.
    """
    from capsem.gate.context import Context, NullJournal

    root = _checkout(tmp_path)
    recorded = gate_config.for_root(root).path(
        gate_config.for_root(root).candidate.source_state_file
    )

    RecordSourceState().perform(Context(Running(root), gate_config.for_root(root)))
    assert not recorded.exists(), "an unrecorded run wrote a source state anyway"

    # And with a real run behind it, it records as before.
    from capsem.gate.runlog import RunLog

    with RunLog.open(gate_config.for_root(root), "candidate") as log:
        RecordSourceState().perform(Context(Running(root), gate_config.for_root(root), journal=log))
    assert json.loads(recorded.read_text(encoding="utf-8"))["head"] == HEAD
    assert isinstance(NullJournal(), NullJournal)


def test_interrogating_the_gate_plan_leaves_the_checkout_alone() -> None:
    """The helper every contract uses, against the real checkout.

    Guarded here rather than in the helper, because the helper is not the only
    thing that will ever run a plan to look at it.
    """
    import sys as _sys

    _sys.path.insert(0, str(PROJECT_ROOT / "tests"))
    from helpers.gate import gate_issued

    config = gate_config.load(PROJECT_ROOT)
    recorded = config.path(config.candidate.source_state_file)
    saved = recorded.read_bytes() if recorded.exists() else None

    # A baseline of this test's own making. Reading whatever happens to be on
    # disk would compare the observer's output against the observer's output
    # the moment a previous run left one there -- and pass.
    sentinel = json.dumps({"head": "sentinel", "digest": "sentinel"}).encode()
    recorded.parent.mkdir(parents=True, exist_ok=True)
    recorded.write_bytes(sentinel)
    try:
        gate_issued("candidate")
        assert recorded.read_bytes() == sentinel, (
            "reading the plan rewrote the gate's own source state"
        )
    finally:
        if saved is None:
            recorded.unlink(missing_ok=True)
        else:
            recorded.write_bytes(saved)
