"""A failed gate should be a directory you can attach, not a scrollback.

Diagnosing one used to mean having been present when it happened. Which command
ran with which arguments, what it exited with, where the forty minutes went,
which bytes came out -- all of it existed only in a terminal, for whoever
happened to be watching it.

Two properties here are not conveniences. Every line is validated on the way
out, because a log anything may append to drifts into a shape nothing can read
back and the first person to notice is the one who needed it. And `exec`
records only the environment a command *added*: this file gets attached to bug
reports, and a release machine's ambient environment holds tokens.
"""

from __future__ import annotations

import json
import os
import threading
from pathlib import Path

import pytest

from capsem.gate import config as gate_config
from capsem.gate.actions import Run
from capsem.gate.errors import GateError
from capsem.gate.execution import step
from capsem.gate.runhistory import read, rotate, runs
from capsem.gate.runlog import RunLog
from capsem.gate.runlogschema import PAYLOADS

PROJECT_ROOT = Path(__file__).resolve().parents[1]
SOURCE = (PROJECT_ROOT / "config" / "gate.toml").read_text(encoding="utf-8")


def _checkout(tmp_path: Path, **overrides: object) -> gate_config.GateConfig:
    """A throwaway checkout whose run-log policy a test can shorten."""
    (tmp_path / "config").mkdir(parents=True, exist_ok=True)
    source = SOURCE
    for key, value in overrides.items():
        original = next(line for line in source.splitlines() if line.startswith(f"{key} = "))
        source = source.replace(original, f"{key} = {value}")
    (tmp_path / "config" / "gate.toml").write_text(source, encoding="utf-8")
    return gate_config.load(tmp_path)


def _events(log: RunLog) -> list[dict]:
    return read(log.directory, log.settings)


# ---------------------------------------------------------------------------
# Every line is a shape something can read back
# ---------------------------------------------------------------------------


def test_every_emitted_line_validates_against_a_model(tmp_path: Path) -> None:
    """The property that makes the log worth writing.

    A log with one unvalidated writer is a log whose reader needs a fallback,
    and the fallback is where the field you wanted turns out to be missing.

    Every registered event is emitted here, and the last assertion is what
    makes that binding. The earlier version wrote five of the eleven and passed
    -- including while `Launch` was emitted by production and registered
    nowhere, because nothing in this file ever launched anything. A model this
    test does not exercise is a model nothing checks.
    """
    config = _checkout(tmp_path)
    with RunLog.open(config, "test", argv=("just", "test")) as log:
        log.shape(("build",), (("build", "test"),))
        with log.step(step("build", Run(["cargo", "build"]))):
            log.note("something worth reading back")
            log.artifact(tmp_path / "vmlinuz", digest="cafe", size=7)
            log.exec(("cargo", "build"), cwd="/src", env={}, exit=0, duration_ms=1.0)
            log.launch(("capsem-service",), cwd="/src", env={}, pid=4242, duration_ms=1.0)
            with log.action(Run(["cargo", "build"])):
                pass
        log.waited("build", dependency_ms=1.0, resource_ms=2.0, execution_ms=3.0)
        log.skipped("never-ran")

    recorded = _events(log)
    assert recorded, "a run that logged nothing is a run nobody can diagnose"

    for entry in recorded:
        payload = {k: v for k, v in entry.items() if k not in {"schema", "ts", "run_id"}}
        model = PAYLOADS[entry["event"]]
        model(**payload)

    assert {entry["event"] for entry in recorded} == set(PAYLOADS), (
        "a registered payload nothing here emits is a payload this test cannot "
        "validate; emit it, or take it out of the registry"
    )


def test_every_line_carries_the_same_envelope(tmp_path: Path) -> None:
    """Added by the writer, so two callers cannot spell it differently."""
    config = _checkout(tmp_path)
    with RunLog.open(config, "test") as log:
        log.note("hello")

    for entry in _events(log):
        assert entry["schema"] == config.runlog.event_schema
        assert entry["run_id"] == log.run_id
        assert entry["ts"] > 0


def test_the_events_are_in_the_order_they_happened(tmp_path: Path) -> None:
    config = _checkout(tmp_path)
    with RunLog.open(config, "test") as log, log.step(step("build", Run(["cargo"]))):
        log.note("during")

    kinds = [entry["event"] for entry in _events(log)]
    assert kinds == ["run.start", "step.start", "note", "step.end", "run.end"]


# ---------------------------------------------------------------------------
# What must not reach the file
# ---------------------------------------------------------------------------


def test_exec_records_only_the_environment_a_command_added(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """A run log is a file people attach to bug reports.

    The ambient environment of a release machine holds tokens, so the whole
    environment is never what gets written -- only the delta the command
    itself declared.
    """
    monkeypatch.setenv("RELEASE_GITHUB_TOKEN", "ghp_do_not_log_me")
    config = _checkout(tmp_path)

    with RunLog.open(config, "test") as log:
        log.exec(
            ("gh", "workflow", "run"),
            cwd="/src",
            env={"CAPSEM_TEST_MODULE": "fast"},
            exit=0,
            duration_ms=2.0,
        )

    body = (log.directory / config.runlog.events).read_text(encoding="utf-8")
    assert "ghp_do_not_log_me" not in body
    assert "CAPSEM_TEST_MODULE" in body


# ---------------------------------------------------------------------------
# Failure
# ---------------------------------------------------------------------------


def test_a_failing_step_is_recorded_with_its_error(tmp_path: Path) -> None:
    config = _checkout(tmp_path)

    with (
        pytest.raises(GateError),
        RunLog.open(config, "test") as log,
        log.step(step("build", Run(["cargo"]))),
    ):
        raise GateError("linker died")

    ended = [e for e in _events(log) if e["event"] == "step.end"]
    assert ended[0]["status"] == "failed"
    assert "linker died" in ended[0]["error"]


def test_the_run_is_closed_even_when_the_body_raises(tmp_path: Path) -> None:
    """An aborted run is exactly the one whose record matters."""
    config = _checkout(tmp_path)

    with pytest.raises(GateError), RunLog.open(config, "test") as log:
        raise GateError("gate exploded")

    ended = [e for e in _events(log) if e["event"] == "run.end"]
    assert ended[0]["status"] == "failed"
    assert "gate exploded" in ended[0]["failures"]["test"]


def test_a_skipped_step_is_not_recorded_as_a_failure(tmp_path: Path) -> None:
    """It never ran. Conflating the two hides how far the real failure went."""
    config = _checkout(tmp_path)
    with RunLog.open(config, "test") as log:
        log.skipped("install")

    ended = [e for e in _events(log) if e["event"] == "step.end"]
    assert ended[0]["status"] == "skipped"


def test_an_interrupt_still_closes_the_run(tmp_path: Path) -> None:
    """Ctrl-C is a path, not an absence of one."""
    config = _checkout(tmp_path)

    with pytest.raises(KeyboardInterrupt), RunLog.open(config, "test") as log:
        raise KeyboardInterrupt

    assert [e for e in _events(log) if e["event"] == "run.end"]


# ---------------------------------------------------------------------------
# Timing and artifacts
# ---------------------------------------------------------------------------


def test_a_step_records_how_long_it_took(tmp_path: Path) -> None:
    """So "the gate is slow" resolves to a line rather than a feeling."""
    config = _checkout(tmp_path)
    with RunLog.open(config, "test") as log, log.step(step("build", Run(["cargo"]))):
        pass

    ended = [e for e in _events(log) if e["event"] == "step.end"]
    assert ended[0]["duration_ms"] >= 0


def test_an_action_is_recorded_by_what_it_would_do(tmp_path: Path) -> None:
    """ "run" as a label says nothing; the whole point of an action being able
    to describe itself is that this line is readable."""
    config = _checkout(tmp_path)
    action = Run(["cargo", "build", "--release"])

    with RunLog.open(config, "test") as log, log.action(action):
        pass

    recorded = [e for e in _events(log) if e["event"] == "action"]
    assert recorded[0]["render"] == "cargo build --release"


def test_a_failing_action_is_recorded_before_the_failure_propagates(
    tmp_path: Path,
) -> None:
    """Otherwise the one line that says which primitive broke is the one line
    the log does not have."""
    config = _checkout(tmp_path)
    action = Run(["cargo", "build"])

    with pytest.raises(GateError), RunLog.open(config, "test") as log, log.action(action):
        raise GateError("linker died")

    recorded = [e for e in _events(log) if e["event"] == "action"]
    assert recorded[0]["status"] == "failed"
    assert recorded[0]["render"] == "cargo build"


def test_a_run_records_the_head_it_qualified(tmp_path: Path) -> None:
    """A timing or artifact comparison against a different revision is not a
    comparison, and the log has to make that checkable."""
    config = _checkout(tmp_path)
    expected = _git_repository(tmp_path)

    with RunLog.open(config, "test") as log:
        pass

    started = next(e for e in _events(log) if e["event"] == "run.start")
    assert started["head"] == expected


def test_a_detached_head_is_recorded_as_the_revision_itself(tmp_path: Path) -> None:
    """A CI checkout is detached, so this is the ordinary release shape."""
    config = _checkout(tmp_path)
    expected = _git_repository(tmp_path, detach=True)

    with RunLog.open(config, "test") as log:
        pass

    started = next(e for e in _events(log) if e["event"] == "run.start")
    assert started["head"] == expected


def _git_repository(root: Path, *, detach: bool = False) -> str:
    """A real repository, because the revision is resolved by asking git.

    These fabricated a `.git` directory with a hand-written HEAD and ref file,
    which is what the old parser read. It could not read a linked worktree or
    a packed ref, and a fabricated tree cannot tell you that.
    """
    import subprocess

    for argv in (
        ["git", "init", "--quiet"],
        ["git", "config", "user.email", "gate@example.test"],
        ["git", "config", "user.name", "Gate"],
        ["git", "config", "commit.gpgsign", "false"],
        ["git", "commit", "--allow-empty", "--quiet", "-m", "root"],
    ):
        subprocess.run(argv, cwd=root, check=True, capture_output=True)
    revision = subprocess.run(
        ["git", "-C", str(root), "rev-parse", "HEAD"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()
    if detach:
        subprocess.run(
            ["git", "-C", str(root), "checkout", "--quiet", "--detach", revision],
            check=True,
            capture_output=True,
        )
    return revision


def test_a_checkout_without_git_still_records_a_run(tmp_path: Path) -> None:
    """A container that unpacked a tarball has no `.git`, and a run there is
    still a run worth having a record of."""
    config = _checkout(tmp_path)

    with RunLog.open(config, "test") as log:
        pass

    started = next(e for e in _events(log) if e["event"] == "run.start")
    assert started["head"] == ""


def test_measuring_a_directory_that_is_gone_is_not_an_error(tmp_path: Path) -> None:
    """Rotation and `gc` both measure trees that another run may have taken."""
    from capsem.gate.runhistory import tree_size

    assert tree_size(tmp_path / "never-existed") == 0


def test_an_artifact_is_recorded_against_the_step_that_made_it(
    tmp_path: Path,
) -> None:
    config = _checkout(tmp_path)
    with RunLog.open(config, "test") as log, log.step(step("kernel", Run(["cargo"]))):
        log.artifact(tmp_path / "vmlinuz", digest="cafe", size=9)

    recorded = [e for e in _events(log) if e["event"] == "artifact"]
    assert recorded[0]["step"] == "kernel"
    assert recorded[0]["digest"] == "cafe"


def test_concurrent_steps_do_not_interleave_their_lines(tmp_path: Path) -> None:
    """The plan runs steps at once, and half a JSON object helps nobody.

    A property of the whole write path rather than of any one guard: today
    atomicity comes from opening, appending one line, and closing, with the
    mutex as insurance. The lines are large on purpose, so this keeps holding
    if that shape ever changes.
    """
    config = _checkout(tmp_path)
    # Well past both PIPE_BUF and Python's own write buffer, so the write
    # reaches the file as several syscalls and a competing thread can land
    # between them.
    long_enough = "x" * 200_000

    with RunLog.open(config, "test") as log:
        writers = [
            threading.Thread(
                target=lambda n=n: [log.note(f"{n}-{i}-{long_enough}") for i in range(6)]
            )
            for n in range(8)
        ]
        for writer in writers:
            writer.start()
        for writer in writers:
            writer.join()

    lines = (log.directory / config.runlog.events).read_text().strip().splitlines()
    assert len(lines) == 8 * 6 + 2, "every note, plus run.start and run.end"
    for line in lines:
        json.loads(line)


# ---------------------------------------------------------------------------
# Finding a run again
# ---------------------------------------------------------------------------


def test_latest_points_at_the_most_recent_run(tmp_path: Path) -> None:
    """One path a bug report can name without knowing the timestamp."""
    config = _checkout(tmp_path)
    with RunLog.open(config, "test"):
        pass
    with RunLog.open(config, "smoke") as second:
        pass

    latest = config.path(config.runlog.root) / config.runlog.latest_link
    assert latest.resolve() == second.directory.resolve()


def test_a_step_gets_its_own_log_so_lanes_stay_readable(tmp_path: Path) -> None:
    """Two build lanes streaming to one terminal interleave into noise."""
    config = _checkout(tmp_path)
    with RunLog.open(config, "test") as log:
        assert log.step_log("assets.arm64") != log.step_log("assets.x86_64")
        assert log.step_log("assets.arm64").parent.is_dir()


# ---------------------------------------------------------------------------
# Rotation
# ---------------------------------------------------------------------------


def _finished_run(root: Path, name: str, *, size: int = 0) -> Path:
    directory = root / name
    directory.mkdir(parents=True)
    (directory / "run.jsonl").write_text(
        '{"event": "run.start"}\n{"event": "run.end"}\n' + "x" * size
    )
    return directory


def _crashed_run(root: Path, name: str) -> Path:
    directory = root / name
    directory.mkdir(parents=True)
    (directory / "run.jsonl").write_text('{"event": "run.start"}\n')
    return directory


def test_rotation_drops_the_oldest_first(tmp_path: Path) -> None:
    config = _checkout(tmp_path, keep_runs=2)
    root = config.path(config.runlog.root)
    for name in ("20260101-000000-test", "20260102-000000-test", "20260103-000000-test"):
        _finished_run(root, name)

    rotate(config)

    assert [entry.name for entry in runs(config)] == [
        "20260103-000000-test",
        "20260102-000000-test",
    ]


def test_rotation_gives_up_completed_runs_before_crashed_ones(
    tmp_path: Path,
) -> None:
    """The run with no `run.end` is the one somebody still wants.

    It is also the one nothing else recorded, since a crashed gate is exactly
    the case where the terminal output was lost with it.
    """
    config = _checkout(tmp_path, keep_runs=1)
    root = config.path(config.runlog.root)
    _crashed_run(root, "20260101-000000-test")
    _finished_run(root, "20260109-000000-test")

    rotate(config)

    assert [entry.name for entry in runs(config)] == ["20260101-000000-test"]


def test_rotation_also_honours_the_byte_cap(tmp_path: Path) -> None:
    """Count alone does not bound a gate that writes gigabytes per run."""
    config = _checkout(tmp_path, keep_runs=50, keep_bytes=2_000)
    root = config.path(config.runlog.root)
    for name in ("20260101-000000-test", "20260102-000000-test", "20260103-000000-test"):
        _finished_run(root, name, size=1_000)

    rotate(config)

    assert len(runs(config)) < 3


def test_rotation_never_removes_the_run_being_written(tmp_path: Path) -> None:
    """A rotation that can delete the directory it is about to write into is a
    rotation with a bad day in it.

    Exercised against `rotate` directly, with the excluded run made the most
    attractive candidate there is: oldest, finished, and the largest thing on
    disk. Driving this through `RunLog.open` would prove nothing -- rotation
    happens before the run has written a byte, so its directory is empty and
    never trips the cap however the ranking is written.
    """
    config = _checkout(tmp_path, keep_runs=2, keep_bytes=1)
    root = config.path(config.runlog.root)
    current = _finished_run(root, "20260101-000000-test", size=50_000)
    _finished_run(root, "20260102-000000-test", size=10)
    _finished_run(root, "20260103-000000-test", size=10)

    rotate(config, keep=current)

    assert current.is_dir(), "the run being written must survive its own rotation"


def test_rotation_leaves_a_usable_latest_link(tmp_path: Path) -> None:
    """A dangling `latest` sends whoever follows it nowhere."""
    config = _checkout(tmp_path, keep_runs=1)
    for command in ("test", "smoke", "assets"):
        with RunLog.open(config, command):
            pass

    latest = config.path(config.runlog.root) / config.runlog.latest_link
    assert latest.resolve().is_dir()


def test_rotation_of_an_empty_root_is_harmless(tmp_path: Path) -> None:
    config = _checkout(tmp_path)

    assert rotate(config) == []
    assert runs(config) == []


def test_a_run_directory_with_no_events_counts_as_unfinished(tmp_path: Path) -> None:
    """A run killed before its first write is the least finished of all, and
    rotation must not read it as complete and give it up first."""
    from capsem.gate.runhistory import finished

    config = _checkout(tmp_path)
    bare = config.path(config.runlog.root) / "20260101-000000-test"
    bare.mkdir(parents=True)

    assert not finished(bare, config.runlog)


def test_reading_a_run_that_wrote_nothing_returns_nothing(tmp_path: Path) -> None:
    config = _checkout(tmp_path)
    empty = config.path(config.runlog.root) / "20260101-000000-test"
    empty.mkdir(parents=True)

    assert read(empty, config.runlog) == []


# ---------------------------------------------------------------------------
# What a run says about itself
# ---------------------------------------------------------------------------


def test_a_run_records_enough_to_tell_whether_two_are_comparable(
    tmp_path: Path,
) -> None:
    """A timing comparison between a four-core runner and a laptop is not a
    regression, and the log has to make that visible."""
    config = _checkout(tmp_path)
    with RunLog.open(config, "test", argv=("just", "test")):
        pass

    started = next(e for e in _events(config_log(config)) if e["event"] == "run.start")
    assert started["command"] == "test"
    assert started["argv"] == ["just", "test"]
    assert started["cores"] == os.cpu_count()
    assert started["free_gb"] > 0


def config_log(config: gate_config.GateConfig):
    """The most recent run, as `latest` resolves it."""

    class _Resolved:
        directory = config.path(config.runlog.root) / config.runlog.latest_link
        settings = config.runlog

    return _Resolved()


def test_rotation_does_not_claim_a_run_it_could_not_delete(tmp_path, monkeypatch) -> None:
    """`ignore_errors=True` let retention report reclaimed bytes still on disk.

    A run directory that refuses to go is a real condition -- a file held open,
    a permission the gate does not have -- and reporting it as removed means
    the next capacity decision is made against a number that is wrong.
    """
    from capsem.gate import fileactions, runhistory
    from capsem.gate.errors import GateError

    def refuses(path, *args, **kwargs):
        raise PermissionError(13, "Permission denied", str(path))

    # The real removal refuses, so `remove` has to turn that into a failure
    # rather than swallowing it -- patching `remove` itself would test the
    # patch.
    monkeypatch.setattr(fileactions.shutil, "rmtree", refuses)
    config = _checkout(tmp_path, keep_runs=1)
    root = config.path(config.runlog.root)
    for name in ("20260101-000000-a", "20260102-000000-b"):
        _finished_run(root, name)

    with pytest.raises(GateError, match="Permission denied"):
        runhistory.rotate(config)


# ---------------------------------------------------------------------------
# Which command a run *was*
# ---------------------------------------------------------------------------


def _recorded_argv(raw: list[str], tmp_path: Path) -> tuple[str, ...]:
    """Drive the real CLI construction path, and read back what it recorded."""

    from capsem.gate import cli
    from capsem.gate.command import GateCommand

    parsed = cli.build_parser().parse_args(raw)
    command = GateCommand.registry[parsed.gate_command](
        _Recording(tmp_path), parsed, invocation=cli.invocation(raw)
    )
    return command._argv()


class _Recording:
    """Just enough runner for a command to be constructed."""

    def __init__(self, root: Path) -> None:
        self.root = root


@pytest.mark.parametrize(
    "raw",
    (
        ["release-binaries", "nightly"],
        ["release-profile", "candidate", "code"],
        ["logs", "service"],
        ["candidate", "--timing"],
        ["cross-compile", "arm64"],
        ["exec", "--", "echo hi"],
    ),
)
def test_a_run_records_the_invocation_it_was_given(raw: list[str]) -> None:
    """`release-binaries` recorded `('release-binaries',)` -- no channel.

    The namespace was searched for a field named `argv` that almost no command
    defines, so every positional and every flag was dropped. Reading a failed
    release back, there was no way to tell which channel it had attempted.
    """
    assert _recorded_argv(raw, PROJECT_ROOT) == ("capsem-gate", *raw)


def test_the_recorded_invocation_reaches_the_run_start_event(tmp_path: Path) -> None:
    config = _checkout(tmp_path)
    invocation = ("capsem-gate", "release-binaries", "nightly")

    with RunLog.open(config, "release-binaries", argv=invocation) as log:
        directory = log.directory

    (start,) = [e for e in read(directory, config.runlog) if e["event"] == "run.start"]
    assert tuple(start["argv"]) == invocation


# ---------------------------------------------------------------------------
# The two windows around a run's own directory
# ---------------------------------------------------------------------------
#
# Both are reproduced by starting a second allocator *inside* the window, with
# a retention budget of one byte so anything it is allowed to drop, it drops.
# The existing multiprocessing test begins asserting only after `RunLog.open`
# has returned, which is after both windows have closed.


def _tight(tmp_path: Path):
    """A checkout whose retention will delete anything it may."""
    return _checkout(tmp_path, keep_runs=1, keep_bytes=1)


def _second_allocator(config) -> threading.Thread:
    """Another command opening its own run, as a thread we can join."""

    def allocate() -> None:
        with RunLog.open(config, "other"):
            pass

    return threading.Thread(target=allocate)


def test_a_run_is_protected_before_its_directory_becomes_visible(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The directory was created, and only then marked live.

    An allocator arriving in that window saw a directory with no `run.end` and
    no held marker -- indistinguishable from a run that crashed -- and under
    any real budget took it. Allocation happens under the history lock now, so
    the assertion is that a second allocator *cannot even start* while this
    run is becoming visible.
    """
    config = _tight(tmp_path)
    from capsem.gate import runlog as module

    real = module.hold_active
    observed: list[bool] = []

    def barrier(directory: Path, settings):
        # Fires for this run only: the second allocator reaches the same code
        # and would otherwise start a third, and so on.
        if not observed:
            thread = _second_allocator(config)
            thread.start()
            thread.join(timeout=1.0)
            observed.append(thread.is_alive())
            monkeypatch.setattr(module, "hold_active", real)
            thread.join(timeout=10)
        return real(directory, settings)

    monkeypatch.setattr(module, "hold_active", barrier)

    with RunLog.open(config, "mine") as log:
        directory = log.directory

    assert observed == [True], (
        "another allocator ran while this directory existed unmarked, which is "
        "the window it used to be rotated away in"
    )
    assert directory.is_dir()
    events = {entry["event"] for entry in read(directory, config.runlog)}
    assert {"run.start", "run.end"} <= events


def test_a_run_stays_protected_until_its_summary_is_written(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The marker was released before `run.end` and the summary were written.

    A waiting command could therefore rotate the just-finished directory while
    its owner was still writing into it -- which under a byte cap turns an
    otherwise successful release into a logging failure after publication.
    """
    config = _tight(tmp_path)
    from capsem.gate import runlog as module

    real = module.write_summary
    ran: list[bool] = []

    def barrier(directory: Path, settings, **kwargs):
        if not ran:
            monkeypatch.setattr(module, "write_summary", real)
            thread = _second_allocator(config)
            thread.start()
            thread.join(timeout=10)
            ran.append(not thread.is_alive())
        return real(directory, settings, **kwargs)

    monkeypatch.setattr(module, "write_summary", barrier)

    with RunLog.open(config, "mine") as log:
        directory = log.directory

    assert ran == [True], "the second allocator never finished"
    assert directory.is_dir(), "the just-finished run was rotated while closing"
    assert (directory / config.runlog.summary).is_file()


def test_retention_measures_each_remaining_run_once(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """It re-walked every surviving tree on every removal pass.

    Same answer each time -- a directory that was not removed has not changed
    size -- at a cost that grows with runs times files.
    """
    from capsem.gate import runhistory

    config = _checkout(tmp_path, keep_runs=1, keep_bytes=1)
    root = config.path(config.runlog.root)
    for name in ("20260101-000001-aaa-x", "20260101-000002-bbb-x", "20260101-000003-ccc-x"):
        directory = root / name
        directory.mkdir(parents=True)
        (directory / config.runlog.events).write_text('{"event": "run.end"}\n', encoding="utf-8")

    measured: list[Path] = []
    real = runhistory.tree_size
    monkeypatch.setattr(
        runhistory, "tree_size", lambda path: (measured.append(path), real(path))[1]
    )

    runhistory.rotate(config)

    assert len(measured) == len(set(measured)), f"measured twice: {measured}"
