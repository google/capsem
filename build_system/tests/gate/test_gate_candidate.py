"""The three guarantees `just test-clean` makes on the way out.

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
import os
from pathlib import Path

import pytest
from capsem_builder.gate import cli  # noqa: F401 - imported so every command registers
from capsem_builder.gate import config as gate_config
from capsem_builder.gate.candidate import CandidateCommand, keep_awake
from capsem_builder.gate.command import GateCommand
from capsem_builder.gate.context import Context
from capsem_builder.gate.errors import GateError
from capsem_builder.gate.lifecycle import held
from capsem_builder.gate.sourcestate import (
    RecordSourceState,
    RequireIsolatedBytecode,
    RequireSourceUnchanged,
)
from helpers import gate as gate_helpers
from helpers.gate import RecordingJournal, RecordingRunner, gate_issued

# The Colima lifecycle has its own home in
# build_system/tests/gate/test_colima_lifecycle.py, where it is driven
# against a real executable rather than a recording runner.

PROJECT_ROOT = Path(__file__).resolve().parents[3]


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
    """A context for a gate that is really running.

    `observing` defaults false, which is the point: these tests are about what
    a run does to the machine (see `test_an_observed_plan_touches_nothing`).
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
    from capsem_builder.gate.gateresources import OrphanAccounting

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
    from capsem_builder.gate.gateresources import FailureEvidence

    root = _checkout(tmp_path)
    RecordSourceState().perform(_context(root))
    runner = Running(root)
    evidence = FailureEvidence(gate_config.for_root(root), runner)

    with pytest.raises(GateError), held(evidence):
        raise GateError("boom")

    captured = runner.matching(r"capture-failure")
    assert captured
    assert HEAD[:12] in captured[0]


def test_release_failure_evidence_keeps_attempt_and_source_identities(
    tmp_path: Path,
) -> None:
    from capsem_builder.gate.gateresources import FailureEvidence

    root = _checkout(tmp_path)
    config = gate_config.for_root(root)
    selected = "1" * 40
    recorded = config.path(config.candidate.source_state_file)
    recorded.parent.mkdir(parents=True, exist_ok=True)
    recorded.write_text(
        json.dumps({"source_kind": "commit", "source_commit": selected, "head": selected})
    )

    class ReleaseRun(Running):
        @property
        def run_id(self) -> str:
            return "20260813-010203-abcdef-release-binaries"

    runner = ReleaseRun(root)
    with pytest.raises(GateError), held(FailureEvidence(config, runner)):
        raise GateError("boom")

    (captured,) = runner.matching(r"capture-failure")
    assert "--run-id 20260813-010203-abcdef-release-binaries" in captured
    assert f"--source-commit {selected}" in captured


def test_a_passing_run_captures_no_failure_evidence(tmp_path: Path) -> None:
    from capsem_builder.gate.gateresources import FailureEvidence

    root = _checkout(tmp_path)
    runner = Running(root)

    with held(FailureEvidence(gate_config.for_root(root), runner)):
        pass

    assert not runner.ran(r"capture-failure")


def test_the_gate_holds_everything_that_must_be_given_back() -> None:
    """The set, so a later change cannot quietly drop one."""
    names = {resource.name for resource in _command(PROJECT_ROOT).resources(RUNNER_FOR_RESOURCES)}

    assert names == {
        "orphan-accounting",
        "sandbox-report",
        "release-egress",
        "workspace",
        "colima",
        "failure-evidence",
    }


# ---------------------------------------------------------------------------
# Staying awake
# ---------------------------------------------------------------------------


def test_macos_wraps_the_gate_so_the_machine_cannot_sleep_through_it(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr("capsem_builder.gate.host.system", lambda: "Darwin")
    monkeypatch.setattr("shutil.which", lambda _name: "/usr/bin/caffeinate")
    monkeypatch.delenv(CONFIG.candidate.keep_awake_marker, raising=False)

    prefix = keep_awake(Running(_checkout(tmp_path)))

    assert prefix is not None
    assert prefix[0] == "caffeinate"
    assert f"{CONFIG.candidate.keep_awake_marker}=1" in prefix


def test_the_wrapper_is_applied_once(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    """Without the marker it would re-exec itself forever."""
    monkeypatch.setattr("capsem_builder.gate.host.system", lambda: "Darwin")
    monkeypatch.setenv(CONFIG.candidate.keep_awake_marker, "1")

    assert keep_awake(Running(_checkout(tmp_path))) is None


def test_linux_needs_no_wrapper(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr("capsem_builder.gate.host.system", lambda: "Linux")

    assert keep_awake(Running(_checkout(tmp_path))) is None


def test_a_macos_host_without_caffeinate_is_told_why_it_matters(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr("capsem_builder.gate.host.system", lambda: "Darwin")
    monkeypatch.setattr("shutil.which", lambda _name: None)
    monkeypatch.delenv(CONFIG.candidate.keep_awake_marker, raising=False)

    with pytest.raises(GateError, match="unattended"):
        keep_awake(Running(_checkout(tmp_path)))


# ---------------------------------------------------------------------------
# Observing a plan is not running one
# ---------------------------------------------------------------------------


def test_an_observed_plan_touches_nothing(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    """`tests/helpers/gate.py` runs the real candidate plan against a
    recording runner to read back the argv it would issue. That stubs
    subprocesses -- it does not stub filesystem actions, so `source.record`
    wrote the gate's own `target/gate-source-state.json` with the recorder's
    empty output, and the gate's `source.verify` then reported

        source HEAD changed while the gate was running:  -> <head>

    for a tree nobody had touched. It only reached that far once the last step
    in the plan started passing.

    Reading a plan is not running one, and the context says so rather than
    each action deciding for itself -- the next action to write a file will not
    remember either.
    """
    root = _checkout(tmp_path)
    config = gate_config.for_root(root)
    recorded = config.path(config.candidate.source_state_file)

    observed = Context(Running(root), config, journal=RecordingJournal(), observing=True)
    RecordSourceState().perform(observed)
    assert not recorded.exists(), "an observed plan wrote to the checkout"

    # An inspector is not executing the gate, so requiring its launcher marker
    # would stop observation at source.record and hide every later command.
    from capsem_builder.gatelaunch import MARKER

    monkeypatch.delenv(MARKER, raising=False)
    RequireIsolatedBytecode().perform(observed)

    # And a run that is really running records exactly as before.
    RecordSourceState().perform(_context(root))
    assert json.loads(recorded.read_text(encoding="utf-8"))["head"] == HEAD


def test_observation_reaches_past_a_step_that_claims_an_output(
    tmp_path: Path,
) -> None:
    """Otherwise it stops at the first one and every later step goes unseen.

    A step's declared artifacts are hashed after its actions run, and nothing
    built them because nothing ran -- so `Hash` raised `cannot hash ...: it is
    not a file` and the observation ended three steps in. Every contract that
    reads back issued argv was reading a prefix.
    """
    from capsem_builder.gate.execution import step
    from capsem_builder.gate.fileactions import MakeDir

    absent = tmp_path / "never-built.bin"
    journal = RecordingJournal()
    context = Context(
        Running(tmp_path),
        gate_config.for_root(_checkout(tmp_path)),
        journal=journal,
        observing=True,
    )

    step("claims", MakeDir(tmp_path / "made"), produces=(absent,)).run(context)

    assert not (tmp_path / "made").exists(), "an observed step created a directory"
    assert journal.artifacts == [], "an observed step recorded bytes nobody produced"


def test_interrogating_the_gate_plan_leaves_the_checkout_alone() -> None:
    """The helper every contract uses, against the real checkout.

    Guarded here rather than in the helper, because the helper is not the only
    thing that will ever run a plan to look at it.

    The sentinel is this test's own baseline. Reading whatever happens to be on
    disk would compare the observer's output against the observer's output the
    moment a previous run left one there -- and pass. It matters most during a
    real gate, when the file already holds the true HEAD: a broken `observing`
    would then rewrite identical bytes and go unnoticed.

    It is written to a *private* path, and that is not incidental. Planting the
    sentinel in the real `target/gate-source-state.json` made this test the one
    thing in the suite that deliberately writes a file the whole suite shares.
    Under `pytest -n 4 --dist=loadfile` that raced: this test wrote the
    sentinel, a test in another worker had snapshotted the file before that
    write, and `conftest._the_running_gate_keeps_its_own_source_state` blamed
    *that* test for the change and restored the file underneath this one. Both
    failed, in `functional.pytest.broad.code`, and neither was at fault. With
    the sentinel private, the guard's invariant -- nothing in the suite writes
    these paths -- is true again, so a future writer is correctly blamed.
    """
    config = gate_config.load(PROJECT_ROOT)
    # Inside `target/`, so it is build output rather than tracked source, and
    # per-process, so four xdist workers cannot collide on it either.
    probe = f"{config.candidate.source_state_file}.probe-{os.getpid()}"
    recorded = config.path(probe)
    shared = config.path(config.candidate.source_state_file)
    shared_before = shared.read_bytes() if shared.exists() else None

    sentinel = json.dumps({"head": "sentinel", "digest": "sentinel"}).encode()
    recorded.parent.mkdir(parents=True, exist_ok=True)
    recorded.write_bytes(sentinel)
    try:
        gate_issued("candidate")

        assert recorded.read_bytes() == sentinel, (
            "reading the plan rewrote the gate's own source state"
        )
        assert (shared.read_bytes() if shared.exists() else None) == shared_before, (
            "reading the plan touched the state file belonging to the gate "
            "running this suite, which is the file every other test shares"
        )
    finally:
        recorded.unlink(missing_ok=True)


def test_exact_release_dispatcher_inspects_its_immutable_prefix_without_candidate_receipt(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    root = _checkout(tmp_path)
    config = gate_config.for_root(root)
    commit = "a" * 40
    validated = []
    monkeypatch.setattr(gate_helpers, "PROJECT_ROOT", root)
    monkeypatch.setenv(config.locks.gate.run_marker, "test-gate")
    monkeypatch.setenv(config.environment.qualified_source_commit, commit)
    monkeypatch.setattr(
        "capsem_builder.gate.sourcecommit.require_detached_checkout",
        lambda found_root, found_commit: validated.append((found_root, str(found_commit))),
    )

    assert gate_helpers._inspection_subject(root, config) == root
    assert validated == [(root, commit)]


def test_exact_plan_inspection_derives_from_the_recorded_source_snapshot(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Parallel contracts must not resnapshot the active exact-run root.

    ``source.record`` has already frozen and verified the only source subject
    an exact qualification may derive products from.  Re-reading the live
    root here made plan inspection vulnerable to transient runtime entries
    created by another xdist worker and failed one otherwise-green exact gate.
    Synthetic checkouts created by tests remain their own subjects even though
    they inherit the parent gate's run marker.
    """
    from capsem_builder.gate import snapshot, sourcecapture
    from helpers import gate as gate_helpers

    frozen = tmp_path / "frozen-source"
    frozen.mkdir()
    selected = sourcecapture.SourceSnapshot(
        frozen,
        sourcecapture.SourceDigest("a" * 64),
    )
    copied_from: list[Path] = []

    monkeypatch.setenv(CONFIG.locks.gate.run_marker, "capsem-gate candidate")
    monkeypatch.delenv(CONFIG.environment.qualified_source_commit, raising=False)
    monkeypatch.setattr(sourcecapture, "require_recorded", lambda _config: selected)
    monkeypatch.setattr(
        gate_config,
        "load",
        lambda root: CONFIG.model_copy(update={"root": root}),
    )

    def populate_subject(source: Path, target: Path, _config: object) -> None:
        copied_from.append(source)
        target.mkdir(parents=True)

    monkeypatch.setattr(snapshot, "populate_subject", populate_subject)
    monkeypatch.setattr(gate_helpers, "_seed_observed_source", lambda _checkout: None)

    with gate_helpers._inspection_checkout(PROJECT_ROOT):
        pass

    synthetic = tmp_path / "synthetic-checkout"
    synthetic.mkdir()
    with gate_helpers._inspection_checkout(synthetic):
        pass

    assert copied_from == [frozen, synthetic]


def test_issued_command_introspection_cannot_clear_live_asset_outputs(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Opaque callbacks run only in an expendable copy while a plan is read.

    ``gate_issued`` used a recording subprocess runner against the checkout it
    was inspecting.  That records ``Run`` actions safely, but an opaque
    ``Call`` executes Python directly.  The assets preflight therefore cleared
    the live ``target/ironbank-assets`` tree halfway through broad pytest,
    deleting the exact profile catalog the same pytest process was consuming.
    """
    from capsem_builder.gate import snapshot

    checkout = tmp_path / "checkout"
    snapshot.populate(PROJECT_ROOT, checkout, gate_config.load(PROJECT_ROOT))
    sentinel = checkout / CONFIG.assets.test_root / "code" / "config" / "profiles" / "proof"
    sentinel.mkdir(parents=True)
    marker = sentinel / "profile.toml"
    marker.write_text('id = "proof"\n', encoding="utf-8")
    inspections = tmp_path / "inspections"
    inspections.mkdir()
    uv_cache = tmp_path / "empty-uv-cache"
    uv_cache.mkdir()
    monkeypatch.setattr("helpers.gate.tempfile.tempdir", str(inspections))
    monkeypatch.setenv("UV_CACHE_DIR", str(uv_cache))
    monkeypatch.setenv("UV_OFFLINE", "1")

    gate_issued("assets", root=checkout)

    assert marker.read_text(encoding="utf-8") == 'id = "proof"\n'
    assert not (checkout / ".venv").exists(), "inspection synced a project environment"
    assert not list(inspections.iterdir()), "the expendable inspection checkout was retained"
