"""What a run log has to get right to be worth reading after a failure.

Each of these is a way the record could disagree with what happened, which is
worse than having no record: an operator trusts it.

Step attribution was one mutable string on the `RunLog`. The plan runs
independent steps concurrently, so whichever step set it last owned every
action, note, artifact and subprocess emitted by any of them until the next
one moved it. Each write was mutex-protected, which made the *lines* fine and
the *attribution* wrong.

Timing classified a run by its steps alone, so a failure outside any step --
the machine lock, a resource that would not acquire, a teardown that raised --
produced a report saying `ok` about a run that failed.

And `runs` recorded itself: asking which run failed opened a new run and
repointed `latest` at the question, so `runs last` could answer with the query.
"""

from __future__ import annotations

import argparse
import json
import threading
from pathlib import Path

import pytest
from capsem_builder.gate import cli  # noqa: F401 - imported so every command registers
from capsem_builder.gate import config as gate_config
from capsem_builder.gate.actions import Action
from capsem_builder.gate.command import GateCommand
from capsem_builder.gate.context import Context
from capsem_builder.gate.errors import GateError
from capsem_builder.gate.execution import step
from capsem_builder.gate.plan import Plan
from capsem_builder.gate.runlog import RunLog
from helpers.gate import RecordingRunner

PROJECT_ROOT = Path(__file__).resolve().parents[3]


def _checkout(tmp_path: Path) -> gate_config.GateConfig:
    (tmp_path / "config").mkdir(parents=True)
    (tmp_path / "config" / "gate.toml").write_text(
        (PROJECT_ROOT / "config" / "gate.toml").read_text(encoding="utf-8"),
        encoding="utf-8",
    )
    return gate_config.load(tmp_path)


def _events(log: RunLog) -> list[dict]:
    source = log.directory / log.settings.events
    return [json.loads(line) for line in source.read_text().splitlines()]


class _Noting(Action, name="noting"):
    """Reports its own label, after waiting for its sibling to start."""

    def __init__(self, label: str, gate: threading.Barrier) -> None:
        self._label = label
        self._gate = gate

    def render(self) -> str:
        return f"note {self._label}"

    def perform(self, context: Context) -> None:
        # Both steps are inside their own `journal.step` before either emits.
        # Without that the interleaving this is about may not happen at all.
        self._gate.wait(timeout=5)
        context.journal.note(self._label)


# ---------------------------------------------------------------------------
# Attribution under concurrency
# ---------------------------------------------------------------------------


def test_concurrent_steps_do_not_steal_each_others_events(tmp_path: Path) -> None:
    """One mutable `_current` meant the last step to start owned everything.

    The plan runs whatever the graph makes simultaneously ready, so this is the
    normal case rather than an edge one -- and the failure mode is a run log
    that confidently attributes a failure to the wrong step.
    """
    config = _checkout(tmp_path)
    gate = threading.Barrier(2)
    plan = Plan("concurrent")
    plan.add(step("left", _Noting("left", gate)))
    plan.add(step("right", _Noting("right", gate)))

    with RunLog.open(config, "concurrent") as log:
        plan.run(Context(RecordingRunner(tmp_path), config, journal=log))
        notes = {
            event["step"]: event["message"]
            for event in _events(log)
            if event["event"] == "note"
        }

    assert notes == {"left": "left", "right": "right"}


def test_a_subprocess_is_attributed_to_the_step_that_ran_it(tmp_path: Path) -> None:
    """The same hazard for `exec`, which the runner emits from another thread."""
    from capsem_builder.gate.actions import Run
    from capsem_builder.gate.funnel import GuardedRunner

    config = _checkout(tmp_path)
    plan = Plan("attributed")
    plan.add(step("alpha", Run(["cargo", "build"])))
    plan.add(step("beta", Run(["cargo", "clippy"])))

    with RunLog.open(config, "attributed") as log:
        runner = GuardedRunner(RecordingRunner(tmp_path), journal=log)
        plan.run(Context(runner, config, journal=log))
        execs = {
            " ".join(event["argv"]): event["step"]
            for event in _events(log)
            if event["event"] == "exec"
        }

    assert execs == {"cargo build": "alpha", "cargo clippy": "beta"}


# ---------------------------------------------------------------------------
# What the record says about the run as a whole
# ---------------------------------------------------------------------------


def test_a_run_that_failed_outside_any_step_is_not_reported_as_ok(
    tmp_path: Path,
) -> None:
    """Lock, acquire, release and teardown failures all live outside a step.

    Classifying by steps alone meant a run whose every step passed and whose
    workspace then refused to release was reported as a success.
    """
    from capsem_builder.gate.timing import measure

    config = _checkout(tmp_path)
    with pytest.raises(GateError, match="teardown"), RunLog.open(config, "outside"):
        raise GateError("teardown broke")

    directory = config.path(config.runlog.root) / config.runlog.latest_link
    events = [json.loads(line) for line in (directory / config.runlog.events).read_text().splitlines()]

    assert measure(events).outcome == "failed"


def test_a_run_that_passed_is_reported_as_such(tmp_path: Path) -> None:
    """The other half, so the check above cannot pass by always failing."""
    from capsem_builder.gate.timing import measure

    config = _checkout(tmp_path)
    with RunLog.open(config, "clean") as log:
        pass

    assert measure(_events(log)).outcome == "ok"


def test_the_summary_is_written_where_a_bug_report_can_find_it(
    tmp_path: Path,
) -> None:
    """A run directory you attach, not a scrollback you had to be present for."""
    config = _checkout(tmp_path)
    with RunLog.open(config, "summarised") as log:
        directory = log.directory

    assert (directory / config.runlog.summary).is_file()


# ---------------------------------------------------------------------------
# Asking a question must not become part of the answer
# ---------------------------------------------------------------------------


@pytest.mark.parametrize(
    ("command", "args"),
    [("runs", {}), ("gc", {"dry_run": True, "aggressive": False})],
)
def test_inspecting_runs_does_not_create_one(command: str, args: dict) -> None:
    """`runs last` opened a run and repointed `latest` at itself first.

    So the honest answer to "which run failed" could be the question.

    `gc` is here only in its `--dry-run` shape. It used to be listed outright,
    which is how a command that reclaims whole trees came to be classified
    with the run readers and approved for silence by name.
    """
    import argparse

    from helpers.gate import RecordingRunner

    instance = GateCommand.registry[command](
        RecordingRunner(PROJECT_ROOT),
        argparse.Namespace(
            **{"dry_run": False, "graph": False, "timing": False, **args}
        ),
    )
    assert instance.should_record() is False


def test_every_command_that_changes_something_still_records(tmp_path: Path) -> None:
    """Non-recording is for inspection, and must not spread quietly.

    Asked of an invocation rather than a class, because whether a command
    records can depend on how it was called -- which is exactly the
    distinction `gc` needed and did not have.
    """
    import argparse

    from helpers.gate import RecordingRunner

    silent = []
    for name, command in GateCommand.registry.items():
        if not command.__module__.startswith("capsem_builder.gate."):
            continue
        extra = {"aggressive": False} if name == "gc" else {}
        try:
            instance = command(
                RecordingRunner(PROJECT_ROOT),
                argparse.Namespace(
                    dry_run=False, graph=False, timing=False, **extra
                ),
            )
        except TypeError:
            continue
        if not instance.should_record():
            silent.append(name)

    assert sorted(silent) == ["runs", "version"], (
        "a normal gc reclaims disk, so it is not inspection"
    )


# ---------------------------------------------------------------------------
# Identity
# ---------------------------------------------------------------------------


def test_two_runs_started_in_the_same_second_get_different_directories(
    tmp_path: Path,
) -> None:
    """Run ids had one-second resolution, and the gate lock is taken *after*
    the log is opened -- so two contenders could collide on the way in."""
    config = _checkout(tmp_path)

    with RunLog.open(config, "same") as first, RunLog.open(config, "same") as second:
        assert first.directory != second.directory


def test_the_recorded_revision_survives_a_linked_worktree(tmp_path: Path) -> None:
    """`.git` is a *file* in a linked worktree, and loose refs are not the only
    way a revision is stored -- so reading `.git/HEAD` by hand returned nothing
    for exactly the checkouts a release is most likely cut from.

    This took `tmp_path` and never used it: it called `head_revision` on the
    ordinary main checkout, so it passed while naming a case it never created.
    A real linked worktree is three commands.
    """
    import subprocess

    from capsem_builder.gate.runhistory import head_revision

    origin = tmp_path / "origin"
    origin.mkdir()
    for argv in (
        ["git", "init", "--quiet"],
        ["git", "config", "user.email", "gate@example.test"],
        ["git", "config", "user.name", "Gate"],
        # A developer's global config may sign commits, which a throwaway
        # fixture has no key for.
        ["git", "config", "commit.gpgsign", "false"],
        ["git", "commit", "--allow-empty", "--quiet", "-m", "root"],
    ):
        subprocess.run(argv, cwd=origin, check=True, capture_output=True)

    linked = tmp_path / "linked"
    subprocess.run(
        ["git", "worktree", "add", "--quiet", "--detach", str(linked)],
        cwd=origin,
        check=True,
        capture_output=True,
    )
    assert (linked / ".git").is_file(), "a linked worktree's .git is a file"

    expected = subprocess.run(
        ["git", "-C", str(linked), "rev-parse", "HEAD"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()

    assert head_revision(linked) == expected


def test_the_recorded_revision_survives_a_packed_ref(tmp_path: Path) -> None:
    """A ref that has been packed away has no loose file to read."""
    import subprocess

    from capsem_builder.gate.runhistory import head_revision

    root = tmp_path / "packed"
    root.mkdir()
    for argv in (
        ["git", "init", "--quiet"],
        ["git", "config", "user.email", "gate@example.test"],
        ["git", "config", "user.name", "Gate"],
        # A developer's global config may sign commits, which a throwaway
        # fixture has no key for.
        ["git", "config", "commit.gpgsign", "false"],
        ["git", "commit", "--allow-empty", "--quiet", "-m", "root"],
        ["git", "pack-refs", "--all"],
    ):
        subprocess.run(argv, cwd=root, check=True, capture_output=True)

    expected = subprocess.run(
        ["git", "-C", str(root), "rev-parse", "HEAD"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()

    assert head_revision(root) == expected


def test_a_checkout_with_no_git_at_all_records_nothing_rather_than_raising(
    tmp_path: Path,
) -> None:
    """A tarball is a real way to receive a source tree."""
    from capsem_builder.gate.runhistory import head_revision

    assert head_revision(tmp_path) == ""


def _command(name: str, **args):
    return GateCommand.registry[name](
        RecordingRunner(PROJECT_ROOT),
        argparse.Namespace(dry_run=False, graph=False, timing=False, **args),
    )
