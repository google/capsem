"""Command execution, and the failure message an operator has to act on."""

from __future__ import annotations

import io
import os
import signal
import subprocess
import sys
import threading
import time
from pathlib import Path

import pytest
from capsem_builder.gate import cancellation, processgroup
from capsem_builder.gate import config as gate_config
from capsem_builder.gate.errors import GateError
from capsem_builder.gate.invocation import Command
from capsem_builder.gate.proc import Runner
from capsem_builder.gate.processgroup import StopPolicy
from helpers.gate import RecordingRunner

PROJECT_ROOT = Path(__file__).resolve().parents[3]
CONFIG = gate_config.load(PROJECT_ROOT)
STOP_POLICY = StopPolicy(
    grace_seconds=CONFIG.execution.cancellation_grace_seconds,
    poll_seconds=CONFIG.execution.cancellation_poll_seconds,
)


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
    (tmp_path / "config").mkdir()
    (tmp_path / "config/gate.toml").write_bytes((PROJECT_ROOT / "config/gate.toml").read_bytes())
    runner = RecordingRunner(tmp_path)

    runner.script("build_system/scripts/build/sync-container-clock.py", "probe")

    assert runner.commands[0].argv == (
        "uv",
        "run",
        "--project",
        "build_system",
        "--frozen",
        "python",
        str(tmp_path / "build_system/scripts/build/sync-container-clock.py"),
        "probe",
    )


def test_the_real_runner_executes_and_captures(tmp_path: Path) -> None:
    """One test against the genuine subprocess path, so the rest may be fakes."""
    runner = Runner(tmp_path, stop_policy=STOP_POLICY)

    assert runner.capture(["printf", "%s", "hello"]) == "hello"
    assert runner.run(["true"]) == 0
    with pytest.raises(GateError):
        runner.run(["false"])


def test_the_real_runner_defaults_the_working_directory_to_the_checkout(
    tmp_path: Path,
) -> None:
    (tmp_path / "marker").write_text("")
    runner = Runner(tmp_path, stop_policy=STOP_POLICY)

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


def _alive(pid: int) -> bool:
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    return True


def _owned_group(pids: Path, *, ignore_term: bool = False) -> tuple[str, ...]:
    disposition = "signal.signal(signal.SIGTERM,signal.SIG_IGN); " if ignore_term else ""
    child = "import signal; " + disposition + "signal.alarm(3); signal.pause()"
    helper = (
        "import os,signal,subprocess,sys; "
        f"{disposition}"
        "signal.alarm(3); "
        f"child=subprocess.Popen([sys.executable,'-c',{child!r}]); "
        "open(sys.argv[1],'w').write(f'{os.getpid()} {child.pid}'); "
        "signal.pause()"
    )
    return (sys.executable, "-c", helper, str(pids))


def _cancel_when_ready(flag: threading.Event, pids: Path) -> threading.Thread:
    def cancel() -> None:
        deadline = time.monotonic() + 2
        while not pids.is_file() and time.monotonic() < deadline:
            time.sleep(0.01)
        flag.set()

    trigger = threading.Thread(target=cancel, daemon=True)
    trigger.start()
    return trigger


def _assert_gone(pids: Path) -> None:
    owned = [int(raw) for raw in pids.read_text().split()]
    deadline = time.monotonic() + 2
    while any(_alive(pid) for pid in owned) and time.monotonic() < deadline:
        time.sleep(0.01)
    assert all(not _alive(pid) for pid in owned), owned


def test_cancelling_a_logged_command_reaps_its_exact_process_group(tmp_path: Path) -> None:
    """A stopped gate must not leave its Docker/build/test descendants behind."""
    pids = tmp_path / "owned-pids"
    unrelated = subprocess.Popen(
        [sys.executable, "-c", "import signal; signal.alarm(10); signal.pause()"],
        start_new_session=True,
    )

    with cancellation.cancellable() as flag:
        trigger = _cancel_when_ready(flag, pids)
        try:
            with pytest.raises(cancellation.Cancelled):
                Runner(PROJECT_ROOT).run(
                    _owned_group(pids),
                    log=tmp_path / "step.log",
                )
            assert unrelated.poll() is None, "cancellation touched an unrelated process group"
        finally:
            trigger.join(timeout=2)
            unrelated.terminate()
            unrelated.wait(timeout=2)

    _assert_gone(pids)
    assert unrelated.returncode == -signal.SIGTERM


@pytest.mark.parametrize("capture", [False, True])
def test_cancellation_reaps_plain_and_captured_commands(tmp_path: Path, capture: bool) -> None:
    pids = tmp_path / f"owned-{capture}"
    with cancellation.cancellable() as flag:
        trigger = _cancel_when_ready(flag, pids)
        with pytest.raises(cancellation.Cancelled):
            runner = Runner(PROJECT_ROOT)
            if capture:
                runner.capture(_owned_group(pids))
            else:
                runner.run(_owned_group(pids))
        trigger.join(timeout=2)
    _assert_gone(pids)


def test_cancellation_escalates_only_its_own_sigterm_resistant_group(tmp_path: Path) -> None:
    pids = tmp_path / "resistant-pids"
    fast = StopPolicy(
        grace_seconds=CONFIG.execution.cancellation_poll_seconds,
        poll_seconds=CONFIG.execution.cancellation_poll_seconds,
    )
    with cancellation.cancellable() as flag:
        trigger = _cancel_when_ready(flag, pids)
        with pytest.raises(cancellation.Cancelled):
            Runner(PROJECT_ROOT, stop_policy=fast).run(
                _owned_group(pids, ignore_term=True),
                log=tmp_path / "resistant.log",
            )
        trigger.join(timeout=2)
    _assert_gone(pids)


def test_cancellation_does_not_adopt_an_unsignalable_process_group(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    def denied(_group: int, _sent: int) -> None:
        raise PermissionError(1, "Operation not permitted")

    monkeypatch.setattr(processgroup.os, "killpg", denied)

    assert not processgroup._group_exists(42)
    processgroup._signal_group(42, signal.SIGKILL)


def test_a_foreground_command_cannot_hide_a_daemon_in_a_new_session(tmp_path: Path) -> None:
    pids = tmp_path / "hidden-daemon"
    child = "import signal; signal.alarm(3); signal.pause()"
    helper = (
        "import os,subprocess,sys,time; "
        f"child=subprocess.Popen([sys.executable,'-c',{child!r}],start_new_session=True); "
        "open(sys.argv[1],'w').write(f'{os.getpid()} {child.pid}'); time.sleep(0.3)"
    )

    # Pinned rather than inherited: the refusal is now switched off where a
    # runner is disposable, so a test that asks for it must say so. Reading the
    # ambient environment made this pass here and fail in CI, which is the
    # inversion that wastes the most time to diagnose.
    strict = StopPolicy(grace_seconds=10.0, poll_seconds=0.1, refuse_survivors=True)
    with pytest.raises(GateError, match="descendants remained") as refused:
        Runner(PROJECT_ROOT, stop_policy=strict).run((sys.executable, "-c", helper, str(pids)))

    # Naming the survivor is the whole use of this guard away from a terminal.
    # It fired once in a release lane and said only that *something* had
    # outlived the command, which left bisecting a 4742-test suite as the way
    # to learn what -- while the guard held the process objects and reported
    # none of them.
    assert "still running:" in str(refused.value)
    assert "signal.pause()" in str(refused.value), (
        f"the surviving command must be identifiable, got: {refused.value}"
    )

    _assert_gone(pids)


def test_a_survivor_is_reported_rather_than_refused_on_a_disposable_runner(
    tmp_path: Path, capfd: pytest.CaptureFixture[str]
) -> None:
    """The machine the guard protects is the one that keeps running.

    A leaked service, VM or container is worth failing for here, where the next
    gate inherits its ports, sockets and locks. A hosted runner is deleted
    minutes later and inherits nothing, so there the same refusal guards a
    machine about to cease to exist -- and it held a binary release twice over
    a process nobody could name.

    Still reaped, and still named, because the leak is a real defect worth
    knowing about. It simply does not fail a release whose artifacts are fine.
    """
    pids = tmp_path / "reported-daemon"
    child = "import signal; signal.alarm(3); signal.pause()"
    helper = (
        "import os,subprocess,sys,time; "
        f"child=subprocess.Popen([sys.executable,'-c',{child!r}],start_new_session=True); "
        "open(sys.argv[1],'w').write(f'{os.getpid()} {child.pid}'); time.sleep(0.3)"
    )
    lenient = StopPolicy(grace_seconds=10.0, poll_seconds=0.1, refuse_survivors=False)

    Runner(PROJECT_ROOT, stop_policy=lenient).run((sys.executable, "-c", helper, str(pids)))

    assert "reaped rather than refused" in capfd.readouterr().err
    _assert_gone(pids)
