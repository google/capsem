"""The one function every command passes through, and what it refuses.

`execute` planned, locked, held and ran -- and trusted each command to log its
own subprocesses, to stay out of its own lock, and to build its plan without
touching anything. None of the three was true.

`RunLog.exec` had no production caller at all, so no run log ever recorded a
single command. Ten plan actions invoked `just` or `capsem-gate`, and because
the machine lock is not reentrant each was a child waiting out a 7200-second
timeout for the lock its own parent was holding. And `plan()` shelled out to
`git rev-parse` on the release path, so `--dry-run` -- the thing you reach for
precisely because you do not want to touch the machine -- touched the machine.

The lesson those three share is that a rule living outside the funnel is a rule
each new command has to remember. These tests put them inside it, where
forgetting is not available.
"""

from __future__ import annotations

import argparse
from contextlib import contextmanager
from pathlib import Path

import pytest
from helpers.gate import RecordingJournal, RecordingRunner

from capsem.gate import config as gate_config
from capsem.gate.actions import Run, Script
from capsem.gate.command import GateCommand
from capsem.gate.errors import GateError
from capsem.gate.execution import step
from capsem.gate.lifecycle import Resource
from capsem.gate.plan import Plan

PROJECT_ROOT = Path(__file__).resolve().parents[1]
CONFIG = gate_config.load(PROJECT_ROOT)


# ---------------------------------------------------------------------------
# Fixtures: a command whose plan is whatever a test needs it to be
# ---------------------------------------------------------------------------


class Recorder(Resource, name="funnel-recorder"):
    """A resource that reports what it exports and whether it was taken."""

    def __init__(self, log: list[str], label: str, **environment: str) -> None:
        self._log, self._label, self._environment = log, label, environment
        self.acquired = False

    def acquire(self) -> None:
        self.acquired = True
        self._log.append(f"acquire {self._label}")

    def release(self) -> None:
        self._log.append(f"release {self._label}")

    def environment(self) -> dict[str, str]:
        return dict(self._environment)


class _Probe(GateCommand, name="funnel-probe", help="a command a test builds"):
    """Its plan and resources are whatever the test assigned before running."""

    steps: tuple = ()
    holdings: tuple[Resource, ...] = ()
    on_plan = None
    replacement: tuple[str, ...] | None = None

    def resources(self) -> tuple[Resource, ...]:
        return self.holdings

    def reexec(self) -> tuple[str, ...] | None:
        return self.replacement

    def plan(self) -> Plan:
        if self.on_plan is not None:
            self.on_plan(self._runner)
        plan = Plan(self.name)
        for item in self.steps:
            plan.add(item)
        return plan


@pytest.fixture
def journal(monkeypatch) -> RecordingJournal:
    """A run log that keeps events in memory instead of under `target/`."""
    recording = RecordingJournal()

    @classmethod
    @contextmanager
    def _open(cls, config, command, *, argv=()):
        yield recording

    monkeypatch.setattr("capsem.gate.runlog.RunLog.open", _open)
    monkeypatch.setattr("capsem.gate.command.RunLog.open", _open)
    return recording


def _probe(runner, **attributes) -> _Probe:
    command = _Probe(runner, argparse.Namespace(dry_run=False, graph=False, timing=False))
    for name, value in attributes.items():
        setattr(command, name, value)
    return command


# ---------------------------------------------------------------------------
# Recursion, which the runner refuses rather than a test noticing
# ---------------------------------------------------------------------------


def test_a_plan_action_may_not_invoke_just(journal) -> None:
    """`just test` from inside a plan is the deadlock, not a style problem.

    The child asks for the machine lock its own parent is holding and waits out
    the full timeout. Ten of these existed, and every one of them looked
    reasonable at the call site.
    """
    runner = RecordingRunner(PROJECT_ROOT)
    command = _probe(runner, steps=(step("qualify", Run(["just", "test"])),))

    with pytest.raises(GateError, match="qualify"):
        command.execute()

    assert not runner.commands, "it must be refused before it runs, not after"


def test_a_plan_action_may_not_invoke_another_gate_command(journal) -> None:
    """Both spellings, because the gate already used both."""
    for argv in (
        ["uv", "run", "capsem-gate", "assets"],
        ["capsem-gate", "assets"],
    ):
        runner = RecordingRunner(PROJECT_ROOT)
        command = _probe(runner, steps=(step("build", Run(argv)),))

        with pytest.raises(GateError, match="build"):
            command.execute()

        assert not runner.commands


def test_re_entry_is_seen_through_a_wrapper(journal) -> None:
    """A wrapper in front is exactly how one of these gets past a naive check.

    `argv[0]` here is `caffeinate`, and the gate really does spell it this way
    on the keep-awake path.
    """
    runner = RecordingRunner(PROJECT_ROOT)
    command = _probe(
        runner,
        steps=(
            step(
                "wrapped",
                Run(["caffeinate", "-dimsu", "env", "MARK=1", "capsem-gate", "candidate"]),
            ),
        ),
    )

    with pytest.raises(GateError, match="wrapped"):
        command.execute()


def test_the_error_says_what_to_do_instead(journal) -> None:
    """A refusal that does not name the alternative just gets worked around."""
    runner = RecordingRunner(PROJECT_ROOT)
    command = _probe(runner, steps=(step("nested", Run(["just", "_sign"])),))

    with pytest.raises(GateError, match="fragment"):
        command.execute()


def test_an_ordinary_program_is_not_mistaken_for_re_entry(journal) -> None:
    """The check must see through `uv run` without swallowing what it wraps.

    `uv run python scripts/...` is how nearly every gate step runs, so a rule
    that flags it is a rule that gets deleted within a day.
    """
    runner = RecordingRunner(PROJECT_ROOT)
    command = _probe(
        runner,
        steps=(
            step(
                "ordinary",
                Run(["uv", "run", "python", "scripts/check-source-syntax.py"]),
                Run(["cargo", "build", "--workspace"]),
                Run(["docker", "run", "--label", "just", "alpine"]),
                Script("scripts/audit-python-lock.sh"),
            ),
        ),
    )

    command.execute()

    assert len(runner.commands) == 4


# ---------------------------------------------------------------------------
# Plans that describe rather than do
# ---------------------------------------------------------------------------


def test_plan_construction_may_not_touch_the_machine(journal) -> None:
    """`--dry-run` is worthless if building the answer runs commands.

    The release plan captured `git rev-parse HEAD` through a fresh runner while
    it was being constructed, so asking what a release *would* do already ran
    something -- and the answer could go stale between the description and the
    execution.
    """
    runner = RecordingRunner(PROJECT_ROOT)
    command = _probe(runner, on_plan=lambda r: r.capture(["git", "rev-parse", "HEAD"]))

    with pytest.raises(GateError, match="plan"):
        command.execute()

    assert not runner.commands


def test_plan_construction_cannot_escape_by_building_its_own_runner(journal) -> None:
    """Sealing the command's runner is not enough, and this is how we know.

    `release.py` built a fresh `Runner(config.root)` inside `plan()` to capture
    `git rev-parse HEAD`. A seal that swapped `self._runner` never saw it: the
    dry run printed a real revision while the recording runner observed no
    commands at all -- the machine touched, and invisibly.

    So the seal is ambient rather than per-instance. Any runner, however it was
    obtained, refuses while a plan is being built.
    """
    from capsem.gate.proc import Runner

    def _own_runner(_ignored) -> None:
        Runner(PROJECT_ROOT).capture(["git", "rev-parse", "HEAD"])

    command = _probe(RecordingRunner(PROJECT_ROOT), on_plan=_own_runner)

    with pytest.raises(GateError, match="plan"):
        command.execute()


@pytest.mark.parametrize("flag", ["dry_run", "graph"])
def test_inspection_issues_no_command_and_acquires_nothing(flag, capsys) -> None:
    """Asking must never become doing.

    `execute` consulted `reexec()` before it looked at these flags, so on macOS
    `capsem-gate candidate --dry-run` re-execed into a real `just test`: a
    supposedly inert question starting a forty-minute destructive gate.
    """
    log: list[str] = []
    runner = RecordingRunner(PROJECT_ROOT)
    resource = Recorder(log, "workspace")
    flags = {"dry_run": False, "graph": False, "timing": False, flag: True}
    command = _Probe(runner, argparse.Namespace(**flags))
    command.steps = (step("build", Run(["cargo", "build"])),)
    command.holdings = (resource,)
    # The exact shape of the regression: a command that would re-exec into
    # something long and destructive must not do so while being asked.
    command.replacement = ("caffeinate", "just", "test")

    command.execute()

    assert not runner.commands, "inspection ran a command"
    assert not log, "inspection acquired a resource"
    assert not resource.acquired
    # It must still answer the question: silence would pass the assertions
    # above for the wrong reason. `--graph` names the steps, `--dry-run` the
    # argv underneath them.
    printed = capsys.readouterr().out
    assert "build" in printed
    if flag == "dry_run":
        assert "cargo build" in printed


# ---------------------------------------------------------------------------
# Auto-logging: no call site is involved
# ---------------------------------------------------------------------------


def test_every_subprocess_is_recorded_once(journal) -> None:
    """The run log's whole purpose, and it had no production caller.

    Recorded here rather than at each call site, because sixteen call sites
    remembering is fifteen chances for one to stop.
    """
    runner = RecordingRunner(PROJECT_ROOT)
    command = _probe(
        runner,
        steps=(
            step("one", Run(["cargo", "build"]), Run(["cargo", "clippy"])),
            step("two", Script("scripts/check-source-syntax.py")),
        ),
    )

    command.execute()

    assert [entry["argv"][:2] for entry in journal.execs] == [
        ("cargo", "build"),
        ("cargo", "clippy"),
        ("uv", "run"),
    ]


def test_a_recorded_command_carries_what_a_diagnosis_needs(journal) -> None:
    runner = RecordingRunner(PROJECT_ROOT)
    command = _probe(runner, steps=(step("one", Run(["cargo", "build"])),))

    command.execute()

    (recorded,) = journal.execs
    assert recorded["exit"] == 0
    assert recorded["cwd"] == str(PROJECT_ROOT)
    assert recorded["duration_ms"] >= 0


def test_a_failing_command_is_recorded_with_its_status(journal) -> None:
    """A run log that only holds successes is a log of the wrong runs."""
    runner = RecordingRunner(PROJECT_ROOT, failures=("cargo build",))
    command = _probe(runner, steps=(step("one", Run(["cargo", "build"])),))

    with pytest.raises(GateError):
        command.execute()

    assert [entry["exit"] for entry in journal.execs] == [1]


def test_the_recorded_environment_is_the_delta_not_the_machines(journal) -> None:
    """This file gets attached to bug reports, and a release machine's
    environment holds tokens."""
    runner = RecordingRunner(PROJECT_ROOT)
    command = _probe(
        runner, steps=(step("one", Run(["cargo", "build"], env={"CAPSEM_MARK": "1"})),)
    )

    command.execute()

    (recorded,) = journal.execs
    assert recorded["env"] == {"CAPSEM_MARK": "1"}


# ---------------------------------------------------------------------------
# Isolation comes from what was acquired, not from what a call site remembered
# ---------------------------------------------------------------------------


def test_the_acquired_resources_environment_reaches_every_command(journal) -> None:
    """`Workspace.environment` existed and production never used it.

    `execute` built a bare `Context`, so every command advertised as isolated
    ran against the developer's own `~/.capsem` -- able to read and modify real
    service state, and able to pass or fail on machine state.
    """
    log: list[str] = []
    runner = RecordingRunner(PROJECT_ROOT)
    command = _probe(
        runner,
        holdings=(Recorder(log, "workspace", CAPSEM_HOME="/tmp/isolated"),),
        steps=(step("one", Run(["cargo", "build"]), Script("scripts/x.py")),),
    )

    command.execute()

    for issued in runner.commands:
        assert issued.env["CAPSEM_HOME"] == "/tmp/isolated"


def test_a_later_resource_wins_over_an_earlier_one(journal) -> None:
    """Acquisition order is the precedence order, as a stack would give."""
    log: list[str] = []
    runner = RecordingRunner(PROJECT_ROOT)
    command = _probe(
        runner,
        holdings=(
            Recorder(log, "outer", CAPSEM_HOME="/tmp/outer", KEEP="yes"),
            Recorder(log, "inner", CAPSEM_HOME="/tmp/inner"),
        ),
        steps=(step("one", Run(["cargo", "build"])),),
    )

    command.execute()

    (issued,) = runner.commands
    assert issued.env["CAPSEM_HOME"] == "/tmp/inner"
    assert issued.env["KEEP"] == "yes"


def test_an_action_that_names_a_variable_still_wins(journal) -> None:
    """The narrower scope is the one that meant it."""
    log: list[str] = []
    runner = RecordingRunner(PROJECT_ROOT)
    command = _probe(
        runner,
        holdings=(Recorder(log, "workspace", CAPSEM_HOME="/tmp/isolated"),),
        steps=(step("one", Run(["cargo", "build"], env={"CAPSEM_HOME": "/tmp/step"})),),
    )

    command.execute()

    assert runner.commands[0].env["CAPSEM_HOME"] == "/tmp/step"


def test_a_failed_acquire_runs_no_command_at_all(journal) -> None:
    """A half-built scope must not execute anything in itself.

    Note what this does *not* prove. `held` yields the resources it acquired
    rather than the ones it was asked for, which reads as the safer shape --
    but the two differ only when an acquire raised, and then the body never
    runs, so no test can tell them apart. The shape is kept for what it says,
    not for a behaviour it defends; the behaviour worth defending is this one.
    """
    log: list[str] = []

    class Refuses(Resource, name="funnel-refuses"):
        def acquire(self) -> None:
            raise GateError("no")

        def release(self) -> None:
            log.append("release refuses")

        def environment(self) -> dict[str, str]:
            return {"CAPSEM_HOME": "/tmp/never"}

    runner = RecordingRunner(PROJECT_ROOT)
    command = _probe(
        runner,
        holdings=(Recorder(log, "workspace", CAPSEM_HOME="/tmp/isolated"), Refuses()),
        steps=(step("one", Run(["cargo", "build"])),),
    )

    with pytest.raises(GateError, match="no"):
        command.execute()

    assert not runner.commands
    assert "release refuses" not in log


# ---------------------------------------------------------------------------
# What a plan must hold before a lock is taken
# ---------------------------------------------------------------------------


def test_a_step_may_not_claim_an_undeclared_exclusive(journal) -> None:
    """`[execution.exclusives]` carries the reason each one exists; a claim on
    something absent from it excludes nothing and reads as though it does."""
    from capsem.gate.harnessschema import Exclusive

    runner = RecordingRunner(PROJECT_ROOT)
    command = _probe(
        runner,
        steps=(
            step(
                "one",
                Run(["cargo", "build"]),
                contends=(Exclusive(name="invented", reason="none"),),
            ),
        ),
    )

    with pytest.raises(GateError, match="invented"):
        command.execute()

    assert not runner.commands, "checked before the machine lock, not after"
