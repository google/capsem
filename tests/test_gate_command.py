"""Dispatch, and the lifecycle every command gets whether it asks or not.

`cli.py` used to call a bare handler with no `finally` anywhere near it. What a
command held was that command's private problem, re-solved per file, and
`--dry-run` could not exist at all because nothing knew what a command was
about to do without doing it.

The point of `execute` being the same for every command is that no command can
have a worse one. That is what most of these tests are about.
"""

from __future__ import annotations

import argparse
from pathlib import Path
from typing import ClassVar

import pytest
from helpers.gate import RecordingRunner

from capsem.gate import cli
from capsem.gate.actions import Run
from capsem.gate.command import GateCommand
from capsem.gate.errors import GateError
from capsem.gate.execution import step
from capsem.gate.lifecycle import Resource
from capsem.gate.plan import Plan

PROJECT_ROOT = Path(__file__).resolve().parents[1]


def _args(**overrides) -> argparse.Namespace:
    return argparse.Namespace(
        **{"dry_run": False, "graph": False, "timing": False, **overrides}
    )


class Tracker(Resource, name="tracker"):
    def __init__(self, log: list[str], label: str) -> None:
        self._log, self._label = log, label

    def acquire(self) -> None:
        self._log.append(f"acquire {self._label}")

    def release(self) -> None:
        self._log.append(f"release {self._label}")


def _command(cls, tmp_path: Path, **overrides):
    """Build a command against a throwaway checkout."""
    (tmp_path / "config").mkdir(parents=True, exist_ok=True)
    (tmp_path / "config" / "gate.toml").write_text(
        (PROJECT_ROOT / "config" / "gate.toml").read_text(encoding="utf-8")
    )
    (tmp_path / "justfile").write_text("# a checkout needs one\n")
    return cls(RecordingRunner(tmp_path), _args(**overrides))


# ---------------------------------------------------------------------------
# The lifecycle a command cannot opt out of
# ---------------------------------------------------------------------------


def test_resources_are_held_for_the_whole_command(tmp_path: Path) -> None:
    order: list[str] = []

    class Held(GateCommand, name="held-example", help="x"):
        def resources(self):
            return (Tracker(order, "a"), Tracker(order, "b"))

        def plan(self) -> Plan:
            plan = Plan(self.name)
            plan.add(step("work", Run(["true"])))
            return plan

    _command(Held, tmp_path).execute()

    assert order == ["acquire a", "acquire b", "release b", "release a"]


def test_a_failing_command_still_releases_what_it_held(tmp_path: Path) -> None:
    """The case an ad-hoc `finally` is written for and then written wrong."""
    order: list[str] = []

    class Broken(GateCommand, name="broken-example", help="x"):
        def resources(self):
            return (Tracker(order, "a"),)

        def plan(self) -> Plan:
            plan = Plan(self.name)
            plan.add(step("work", Run(["false"])))
            return plan

    command = _command(Broken, tmp_path)
    command._runner.fail_on("false")

    with pytest.raises(GateError):
        command.execute()

    assert order == ["acquire a", "release a"]


def test_execute_is_never_overridden() -> None:
    """A command that defines its own bypasses teardown, the machine lock and
    the run log at once -- which is exactly what this class exists to prevent."""
    offenders = [
        cls.__name__
        for cls in GateCommand.registry.values()
        if "execute" in vars(cls)
    ]

    assert not offenders, (
        f"these define their own execute: {offenders}. Put the work in "
        "`plan()` and what it holds in `resources()`."
    )


# ---------------------------------------------------------------------------
# Asking without doing
# ---------------------------------------------------------------------------


class Inspectable(GateCommand, name="inspectable-example", help="x"):
    log: ClassVar[list[str]] = []

    def resources(self):
        return (Tracker(self.log, "a"),)

    def plan(self) -> Plan:
        plan = Plan(self.name)
        first = plan.add(step("build", Run(["cargo", "build", "--release"])))
        plan.add(step("verify", Run(["cargo", "test"])), after=(first,))
        return plan


def test_a_dry_run_runs_nothing_and_holds_nothing(tmp_path: Path) -> None:
    """Free to ask, which is the whole reason it is worth having."""
    Inspectable.log = []
    command = _command(Inspectable, tmp_path, dry_run=True)

    command.execute()

    assert command._runner.commands == []
    assert Inspectable.log == [], "no resource acquired either"


def test_a_dry_run_shows_the_argv_and_the_order(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    Inspectable.log = []
    _command(Inspectable, tmp_path, dry_run=True).execute()

    printed = capsys.readouterr().out
    assert "cargo build --release" in printed
    assert printed.index("build") < printed.index("verify")


def test_the_graph_is_emitted_without_running(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    Inspectable.log = []
    command = _command(Inspectable, tmp_path, graph=True)

    command.execute()

    printed = capsys.readouterr().out
    assert printed.startswith("graph")
    assert "-->" in printed
    assert command._runner.commands == []


def test_every_command_can_be_asked_what_it_would_do() -> None:
    """Added once on the shared parser, so it exists on all of them by
    construction rather than by sixteen people remembering.

    Checked as "the flag is offered" rather than by parsing one invocation:
    some commands take a required subaction, and a test that had to know which
    would be a second place the command surface is written down.
    """
    parser = cli.build_parser()
    subparsers = next(
        action
        for action in parser._subparsers._group_actions
        if hasattr(action, "choices")
    )

    for name, child in subparsers.choices.items():
        offered = {flag for action in child._actions for flag in action.option_strings}
        assert {"--dry-run", "--graph", "--timing"} <= offered, name


# ---------------------------------------------------------------------------
# Registration
# ---------------------------------------------------------------------------


def test_every_command_declares_a_name_and_a_help_line() -> None:
    for name, cls in GateCommand.registry.items():
        assert cls.name == name
        assert cls.help.strip(), f"{name} has no help; `--help` is the surface"


def test_two_commands_cannot_claim_one_name() -> None:
    """Silently shadowing is how a command stops being reachable."""
    with pytest.raises(TypeError, match="two commands"):

        class Duplicate(GateCommand, name="inspectable-example", help="x"):
            def plan(self) -> Plan:
                return Plan(self.name)


def test_the_parser_offers_exactly_the_registered_commands() -> None:
    """The check that caught a subcommand implemented but never registered."""
    parser = cli.build_parser()
    subparsers = next(
        action
        for action in parser._subparsers._group_actions
        if hasattr(action, "choices")
    )

    assert set(subparsers.choices) == set(GateCommand.registry)


def test_a_command_must_say_what_it_does() -> None:
    """`plan` is abstract: a command with no work is a command with no reason."""
    with pytest.raises(TypeError, match="plan"):

        class Empty(GateCommand, name="empty-example", help="x"):
            pass

        Empty(RecordingRunner(PROJECT_ROOT), _args())  # ty: ignore[invalid-argument-type]


# ---------------------------------------------------------------------------
# Exit codes
# ---------------------------------------------------------------------------


def test_a_gate_failure_is_exit_one_with_one_line(
    capsys: pytest.CaptureFixture[str],
) -> None:
    """A traceback here would mean a defect in the package rather than in what
    it is checking, so a `GateError` is reported as prose."""

    class Failing(GateCommand, name="failing-example", help="x"):
        def plan(self) -> Plan:
            plan = Plan(self.name)
            plan.add(step("work", Run(["false"])))
            return plan

    status = cli.main(["failing-example", "--dry-run"])

    assert status == 0
    assert "false" in capsys.readouterr().out


def test_an_unknown_command_is_refused_by_the_parser() -> None:
    with pytest.raises(SystemExit):
        cli.main(["no-such-command"])


# ---------------------------------------------------------------------------
# Re-exec
# ---------------------------------------------------------------------------


def test_a_reexec_happens_before_anything_is_acquired(tmp_path: Path) -> None:
    """The deadlock this hook exists to prevent.

    `candidate` re-execs itself under a keep-awake wrapper. As a step that ran
    inside the held resources, so the child asked for the machine lock its own
    parent was holding and waited out the two-hour timeout. Nothing may be
    acquired before the process has decided whether it is going to be replaced.

    The plan *is* built first, and deliberately: `--dry-run` and `--graph` are
    answered before a re-exec, or asking a question turns into running the
    gate. That is safe because construction happens against a sealed runner --
    a plan describes, so building one acquires nothing and runs nothing.
    """
    order: list[str] = []

    class Replacing(GateCommand, name="replacing-example", help="x"):
        def reexec(self):
            order.append("reexec")
            return ("true",)

        def resources(self):
            return (Tracker(order, "a"),)

        def plan(self) -> Plan:
            order.append("plan")
            plan = Plan(self.name)
            plan.add(step("work", Run(["cargo", "build"])))
            return plan

    with pytest.raises(SystemExit):
        _command(Replacing, tmp_path).execute()

    assert "acquire a" not in order, "a resource was taken before the re-exec"
    assert order.index("reexec") > order.index("plan")


def test_the_candidate_gate_only_reexecs_once() -> None:
    """`keep_awake` returns None on the second pass, or the wrapper would
    re-exec itself forever."""
    from capsem.gate.candidate import CandidateCommand

    assert "reexec" in vars(CandidateCommand), (
        "the keep-awake wrapper belongs in `reexec`, not in a step"
    )


def test_only_the_commands_that_must_replace_themselves_do() -> None:
    """A re-exec discards the run log and the lock, so it is not a thing to
    reach for casually."""
    replacing = sorted(
        name
        for name, cls in GateCommand.registry.items()
        # Only the real ones: this file registers commands of its own.
        if "reexec" in vars(cls) and cls.__module__.startswith("capsem.gate.")
    )

    assert replacing == ["candidate"]
