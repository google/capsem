"""Dispatch, and what the operator sees when a gate step fails.

The shell reported failures as `_pnpm-install failed on line 2234 with exit code
127`. A line number is not a command, an exit code is not a cause, and neither
survives the recipe being edited. Every command here fails with the thing that
failed and no traceback, because a traceback in gate output should mean a defect
in this package rather than a bad checkout.
"""

from __future__ import annotations

from pathlib import Path

import pytest
from helpers.gate import RecordingRunner

from capsem.gate import cli, project_root
from capsem.gate import config as gate_config
from capsem.gate.command import GateCommand
from capsem.gate.context import Context
from capsem.gate.errors import GateError


@pytest.fixture
def recorded(monkeypatch: pytest.MonkeyPatch) -> RecordingRunner:
    """Run the CLI for real, against a runner that records instead of executes."""
    runner = RecordingRunner(project_root(), failures=["rev-parse"])
    monkeypatch.setattr(cli, "Runner", lambda root: runner)
    return runner


def dispatch(argv: list[str], runner: RecordingRunner) -> int:
    """Everything `cli.main` does, short of taking the machine to itself.

    The claim these tests make is that argv reaches the right primitive with
    the right arguments -- which needs the real parser and the real plan, and
    does not need the lock. Calling `cli.main` for an exclusive command took
    it, which is how the gate's own pytest step came to block for two hours on
    the lock its own grandparent was holding.
    """
    args = cli.build_parser().parse_args(argv)
    command = GateCommand.registry[args.gate_command](runner, args)
    command.plan().run(Context(runner, gate_config.load(project_root())))
    return 0


def test_storage_release_reaches_the_policy_script(recorded: RecordingRunner) -> None:
    assert dispatch(["storage", "release", "completed-buildkit-graph"], recorded) == 0

    assert recorded.matching(
        r"docker-storage-policy.py release --boundary after-packages --rail package"
    )


def test_storage_gc_passes_its_rail_through(recorded: RecordingRunner) -> None:
    assert dispatch(["storage", "gc", "--rail", "install"], recorded) == 0

    assert recorded.rendered[0].endswith("gc --rail install")


def test_storage_gc_without_a_rail_omits_the_flag(recorded: RecordingRunner) -> None:
    assert dispatch(["storage", "gc"], recorded) == 0

    assert recorded.rendered[0].endswith("docker-storage-policy.py gc")


def test_storage_clean_carries_scope_and_rail(recorded: RecordingRunner) -> None:
    assert dispatch(["storage", "clean", "--scope", "working", "--rail", "default"], recorded) == 0

    assert recorded.rendered[0].endswith("clean --scope working --rail default")


def test_ensure_space_makes_the_boundary_optional(recorded: RecordingRunner) -> None:
    assert dispatch(["storage", "ensure-space", "package"], recorded) == 0
    assert dispatch(["storage", "ensure-space", "default", "candidate-boundary"], recorded) == 0

    assert recorded.rendered[0].endswith("ensure-docker-space.sh package")
    assert recorded.rendered[1].endswith("ensure-docker-space.sh default candidate-boundary")


def test_version_prints_the_workspace_version(
    recorded: RecordingRunner, capsys: pytest.CaptureFixture[str]
) -> None:
    assert cli.main(["version"]) == 0

    printed = capsys.readouterr().out.strip()
    assert printed.count(".") == 2, printed


def test_stamp_version_runs_against_this_checkout(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    """Dispatched, not reimplemented: the CLI knows only which handler to call."""
    calls: list[Path] = []
    runner = RecordingRunner(project_root())
    monkeypatch.setattr(cli, "Runner", lambda root: runner)
    monkeypatch.setattr(
        "capsem.gate.versions.stamp", lambda root, runner: calls.append(root) or "9.9.9"
    )

    assert dispatch(["stamp-version"], runner) == 0
    assert calls == [project_root()]


# ---------------------------------------------------------------------------
# Failure reporting
# ---------------------------------------------------------------------------


def test_a_gate_error_is_one_line_on_stderr_and_a_nonzero_exit(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    def explode(context):
        raise GateError("missing exact release-mode Debian package")

    monkeypatch.setattr(cli, "Runner", lambda root: RecordingRunner(root))
    monkeypatch.setattr("capsem.gate.versions.workspace_version", explode)

    assert cli.main(["version"]) == 1

    err = capsys.readouterr().err
    assert len(err.strip().splitlines()) == 1, "one line, not a wall"
    assert err.startswith("ERROR: ")
    assert "missing exact release-mode Debian package" in err
    assert "Traceback" not in err


def test_an_interrupt_exits_130_rather_than_reporting_success(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """A gate that reports an interrupted run as a pass is the worst outcome.

    The shell hit exactly this: `$?` inside an EXIT trap is the last command's
    status, which on Ctrl-C is 0, so `exit "$status"` turned an abort into a
    green run.
    """

    def interrupt(_root):
        raise KeyboardInterrupt

    monkeypatch.setattr(cli, "Runner", lambda root: RecordingRunner(root))
    monkeypatch.setattr("capsem.gate.versions.workspace_version", interrupt)

    assert cli.main(["version"]) == 130


def test_an_unknown_command_is_refused_by_the_parser() -> None:
    with pytest.raises(SystemExit) as exit_code:
        cli.main(["conquer-poland"])

    assert exit_code.value.code == 2


def test_no_command_is_refused_rather_than_defaulting() -> None:
    with pytest.raises(SystemExit):
        cli.main([])
