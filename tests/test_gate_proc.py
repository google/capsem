"""Command execution, and the failure message an operator has to act on."""

from __future__ import annotations

import io
from pathlib import Path

import pytest
from helpers.gate import RecordingRunner

from capsem.gate.errors import GateError
from capsem.gate.proc import Command, Runner


def test_a_failing_command_reports_what_ran(tmp_path: Path) -> None:
    """`set -euo pipefail` gave a line number; a line number is not a command."""
    runner = RecordingRunner(tmp_path, failures=["dpkg"])

    with pytest.raises(GateError) as failure:
        runner.run(["dpkg", "-i", "capsem.deb"])

    assert "dpkg -i capsem.deb" in str(failure.value)


def test_a_failing_capture_includes_the_error_output(tmp_path: Path) -> None:
    class Noisy(RecordingRunner):
        def execute(self, command: Command):
            completed = super().execute(command)
            completed.stderr = "no such package"
            return completed

    runner = Noisy(tmp_path, failures=["dpkg-query"])

    with pytest.raises(GateError, match="no such package"):
        runner.capture(["dpkg-query", "-W", "capsem"])


def test_check_false_returns_the_status_instead_of_raising(tmp_path: Path) -> None:
    runner = RecordingRunner(tmp_path, failures=["false"])

    assert runner.run(["false"], check=False) == 1


def test_succeeds_answers_a_probe_without_raising(tmp_path: Path) -> None:
    runner = RecordingRunner(tmp_path, failures=["absent"])

    assert runner.succeeds(["test", "-f", "present"])
    assert not runner.succeeds(["test", "-f", "absent"])


def test_environment_overrides_are_rendered_with_the_command(tmp_path: Path) -> None:
    """A gate log that hides the variables hides why the command behaved."""
    command = Command(argv=("cargo", "build"), env={"RUST_TARGET": "aarch64-unknown-linux-gnu"})

    assert str(command) == "RUST_TARGET=aarch64-unknown-linux-gnu cargo build"


def test_arguments_with_spaces_survive_as_single_arguments(tmp_path: Path) -> None:
    """The shell's quoting problem does not exist here, and must not come back."""
    runner = RecordingRunner(tmp_path)

    runner.run(["cp", "/src/a file.deb", "/dst"])

    assert runner.commands[0].argv == ("cp", "/src/a file.deb", "/dst")
    assert "'/src/a file.deb'" in runner.rendered[0]


def test_script_resolves_against_the_checkout(tmp_path: Path) -> None:
    runner = RecordingRunner(tmp_path)

    runner.script("scripts/docker-storage-policy.py", "gc")

    assert runner.commands[0].argv == (
        "uv",
        "run",
        "python",
        str(tmp_path / "scripts/docker-storage-policy.py"),
        "gc",
    )


def test_the_real_runner_executes_and_captures(tmp_path: Path) -> None:
    """One test against the genuine subprocess path, so the rest may be fakes."""
    runner = Runner(tmp_path)

    assert runner.capture(["printf", "%s", "hello"]) == "hello"
    assert runner.run(["true"]) == 0
    with pytest.raises(GateError):
        runner.run(["false"])


def test_the_real_runner_defaults_the_working_directory_to_the_checkout(
    tmp_path: Path,
) -> None:
    (tmp_path / "marker").write_text("")
    runner = Runner(tmp_path)

    assert runner.capture(["ls"]) == "marker"


def test_index_of_names_the_missing_command_rather_than_returning_nothing(
    tmp_path: Path,
) -> None:
    """A helper that answers -1 turns a missing step into an ordering pass."""
    runner = RecordingRunner(tmp_path)
    runner.run(["true"])

    with pytest.raises(AssertionError, match="no command matched"):
        runner.index_of("docker run")


def test_step_and_note_go_to_the_gate_log(tmp_path: Path) -> None:
    stream = io.StringIO()
    runner = Runner(tmp_path, stream=stream)

    runner.step("Building Linux deb")
    runner.note("reusing dev key")

    assert stream.getvalue() == "=== Building Linux deb ===\nreusing dev key\n"


def test_bash_is_reserved_for_fragments_that_are_genuinely_shell(tmp_path: Path) -> None:
    runner = RecordingRunner(tmp_path)

    runner.bash("ls -t /bundle/deb/*.deb | head -n1")

    assert runner.commands[0].argv == ("bash", "-c", "ls -t /bundle/deb/*.deb | head -n1")
