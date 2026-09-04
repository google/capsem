"""The gate must count processes, not just call cleanup code.

`test_pidfile_cleanup_is_wired.py` proves every pidfile the gate stops is one
some binary writes. That is a claim about wiring. It stayed green through the
entire bug: the pidfile was real, the reaping call was real, and the reap did
nothing because a losing service starter had deleted the file. Six services and
their trays survived a session of release-lane runs, each reparented to launchd,
and every run reported success.

This file guards the other half -- that the gate ends by counting what is still
alive, and that a survivor fails the run.
"""

from __future__ import annotations

import contextlib
import json
import os
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

import pytest
from capsem_builder.gate.tools.ci import check_orphan_processes as ORPHANS
from capsem_builder.gate.tools.ci import run_bounded_command as BOUNDED_MODULE
from helpers.sign import sign_binary

ROOT = Path(__file__).resolve().parents[3]
SCRIPT = ROOT / "build_system" / "scripts" / "ci" / "check-orphan-processes.py"
BOUNDED = ROOT / "build_system" / "scripts" / "ci" / "run-bounded-command.py"
JUSTFILE = ROOT / "justfile"
SEALED_MACOS = (
    sys.platform == "darwin" and os.environ.get("CAPSEM_GATE_COMMAND_SANDBOX_MODE") == "enforce"
)

def _pid_is_alive(pid: int) -> bool:
    result = subprocess.run(
        ["ps", "-p", str(pid), "-o", "stat="],
        check=False,
        capture_output=True,
        text=True,
    )
    return result.returncode == 0 and bool(result.stdout.strip())


def test_direct_development_commands_cannot_wait_on_terminal_stdin() -> None:
    result = subprocess.run(
        [
            sys.executable,
            str(BOUNDED),
            "--timeout-seconds",
            "2",
            "--",
            sys.executable,
            "-c",
            "import sys; raise SystemExit(0 if sys.stdin.read() == '' else 91)",
        ],
        check=False,
        capture_output=True,
        text=True,
    )

    assert result.returncode == 0, result.stderr


def test_bounded_cleanup_tolerates_macos_permission_denied_after_signal(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    calls = 0

    def killpg(_pid: int, _signal: int) -> None:
        nonlocal calls
        calls += 1
        if calls > 1:
            raise PermissionError(1, "Operation not permitted")

    class FinishedProcess:
        pid = 42

        @staticmethod
        def poll() -> int:
            return 0

        @staticmethod
        def wait(timeout: float | None = None) -> int:
            del timeout
            return 0

    monkeypatch.setattr(BOUNDED_MODULE.os, "killpg", killpg)

    BOUNDED_MODULE._terminate_tree(FinishedProcess(), grace_seconds=0)


@pytest.mark.skipif(
    SEALED_MACOS,
    reason="Seatbelt denies ps process inspection; Linux CI executes this proof",
)
def test_direct_development_timeout_reaps_the_complete_process_group(tmp_path: Path) -> None:
    pids = tmp_path / "pids"
    child = (
        "import os,signal,subprocess,sys,time; "
        "p=subprocess.Popen([sys.executable,'-c','import time; time.sleep(300)']); "
        f"open({str(pids)!r},'w').write(f'{{os.getpid()}} {{p.pid}}'); "
        "signal.signal(signal.SIGTERM,lambda *_:(p.wait(),sys.exit(0))); "
        "time.sleep(300)"
    )

    result = subprocess.run(
        [
            sys.executable,
            str(BOUNDED),
            "--timeout-seconds",
            "0.5",
            "--grace-seconds",
            "2",
            "--",
            sys.executable,
            "-c",
            child,
        ],
        check=False,
        capture_output=True,
        text=True,
        timeout=5,
    )

    assert result.returncode == 124
    assert "terminating process group" in result.stderr
    parent_pid, child_pid = (int(value) for value in pids.read_text().split())
    assert not _pid_is_alive(parent_pid)
    assert not _pid_is_alive(child_pid)


@pytest.mark.skipif(
    SEALED_MACOS,
    reason="Seatbelt denies ps process inspection; Linux CI executes this proof",
)
def test_direct_development_wrapper_reaps_a_helper_after_its_leader_exits(
    tmp_path: Path,
) -> None:
    child_pid_file = tmp_path / "child-pid"
    leader = (
        "import subprocess,sys; "
        "p=subprocess.Popen([sys.executable,'-c','import time; time.sleep(300)']); "
        f"open({str(child_pid_file)!r},'w').write(str(p.pid))"
    )

    result = subprocess.run(
        [
            sys.executable,
            str(BOUNDED),
            "--timeout-seconds",
            "2",
            "--grace-seconds",
            "0.2",
            "--",
            sys.executable,
            "-c",
            leader,
        ],
        check=False,
        capture_output=True,
        text=True,
        timeout=5,
    )

    assert result.returncode == 0, result.stderr
    assert not _pid_is_alive(int(child_pid_file.read_text()))


@pytest.mark.skipif(
    SEALED_MACOS,
    reason="Seatbelt denies ps process inspection; Linux CI executes this proof",
)
def test_interrupting_the_wrapper_reaps_its_complete_process_group(tmp_path: Path) -> None:
    pids = tmp_path / "interrupted-pids"
    child = (
        "import os,signal,subprocess,sys,time; "
        "p=subprocess.Popen([sys.executable,'-c','import time; time.sleep(300)']); "
        f"open({str(pids)!r},'w').write(f'{{os.getpid()}} {{p.pid}}'); "
        "signal.signal(signal.SIGTERM,lambda *_:(p.wait(),sys.exit(0))); "
        "time.sleep(300)"
    )
    wrapper = subprocess.Popen(
        [
            sys.executable,
            str(BOUNDED),
            "--timeout-seconds",
            "30",
            "--grace-seconds",
            "2",
            "--",
            sys.executable,
            "-c",
            child,
        ],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    try:
        deadline = time.monotonic() + 2
        while not pids.exists() and time.monotonic() < deadline:
            time.sleep(0.01)
        assert pids.exists(), "the fixture process group never started"

        wrapper.terminate()
        _stdout, stderr = wrapper.communicate(timeout=5)
        assert wrapper.returncode == 128 + signal.SIGTERM, stderr
        parent_pid, child_pid = (int(value) for value in pids.read_text().split())
        assert not _pid_is_alive(parent_pid)
        assert not _pid_is_alive(child_pid)
    finally:
        if wrapper.poll() is None:
            wrapper.kill()
            wrapper.wait()


def _gate_issues(name: str | None = None) -> str:
    """Everything the gate would issue, with real argv. See `helpers.gate`."""
    import sys as _sys

    _sys.path.insert(0, str(ROOT / "tests"))
    from helpers.gate import gate_issues

    return gate_issues(name)


def _candidate_source() -> str:
    return (ROOT / "build_system" / "builder" / "gate" / "candidate.py").read_text(encoding="utf-8")


def _accounting():
    """`OrphanAccounting`, where the order of the count is now decided.

    It was a `run` method whose statement order was the contract, so this read
    the source between two markers. The baseline and the check are a resource's
    `acquire` and `release` now, so the order is the lifecycle's guarantee --
    which is a thing that can be *run* rather than read.
    """
    import sys as _sys

    _sys.path.insert(0, str(ROOT / "tests"))
    from capsem_builder.gate import config as gate_config
    from capsem_builder.gate import sandbox
    from capsem_builder.gate.gateresources import OrphanAccounting, gate_resources
    from helpers.gate import RecordingRunner

    runner = RecordingRunner(ROOT)
    config = gate_config.load(ROOT)
    return (
        OrphanAccounting(config, runner),
        runner,
        gate_resources(config, runner, mode=sandbox.OFF),
    )


# ---------------------------------------------------------------------------
# Wiring: the count has to actually run, and has to be able to fail the gate
# ---------------------------------------------------------------------------
#
# These were assertions about the shell that ran the gate: that the EXIT trap
# was armed, was never disarmed, and used `return "$status"` rather than
# `exit "$status"` -- because `$?` inside a trap is the last command's, which
# on Ctrl-C is 0, so exiting with it turned an abort into a pass.
#
# `capsem_builder.gate.candidate` uses `try`/`finally`, which has no `$?` to misread,
# and `build_system/tests/gate/test_gate_candidate.py` asserts the resulting behaviour directly:
# an interrupted run reports the interrupt, a leaked process fails an
# otherwise-passing run, and a failing run keeps its own error rather than the
# cleanup's. What stays here is that the gate still does this at all.


def test_gate_takes_a_baseline_and_checks_it() -> None:
    accounting, runner, _ = _accounting()

    accounting.acquire()
    baseline_command = " ".join(runner.commands[-1].argv)
    assert "baseline" in baseline_command, (
        "without a baseline the check cannot tell this run's processes from a "
        "developer's own dev daemon, so it can only be reckless or useless"
    )
    assert "cache/state/gate-process-baseline.json" in baseline_command
    accounting.release()
    assert "check" in " ".join(runner.commands[-1].argv)


def test_the_baseline_precedes_anything_that_can_spawn_a_process() -> None:
    """It is the first resource the gate acquires, so nothing the plan runs can
    have spawned a process before the count that will be blamed for it."""
    _, _, resources = _accounting()

    assert type(resources[0]).__name__ == "OrphanAccounting"


def test_the_count_runs_even_when_the_gate_aborts() -> None:
    """An aborted run is the one that skips its cleanup, so it is exactly the
    run whose processes need counting.

    A resource is released on every path out of `held` -- which is what makes
    this a resource and not a step, since a step after a failed step is
    skipped.
    """
    import sys as _sys

    _sys.path.insert(0, str(ROOT / "tests"))
    from capsem_builder.gate.lifecycle import held

    accounting, runner, _ = _accounting()
    with contextlib.suppress(RuntimeError), held(accounting):
        raise RuntimeError("the gate aborted")

    issued = [" ".join(command.argv) for command in runner.commands]
    assert any("baseline" in command for command in issued)
    assert any("check" in command for command in issued)


def test_the_check_still_reaches_the_justfile() -> None:
    """The recipe dispatches; the module decides."""
    assert "capsem-gate candidate" in JUSTFILE.read_text(encoding="utf-8")


# ---------------------------------------------------------------------------
# Classification: who gets blamed
# ---------------------------------------------------------------------------


def _facts(pid: int, created: float, exe: str = "/repo/cache/target/cargo/debug/capsem-service") -> dict:
    return {
        "pid": pid,
        "name": Path(exe).name,
        "cmdline": exe,
        "exe": exe,
        "created": created,
        "ppid": 1,
    }


def test_processes_that_predate_the_run_are_not_blamed() -> None:
    current = {10: _facts(10, 100.0), 11: _facts(11, 200.0)}

    leaked = ORPHANS.started_during_run(current, {10: 100.0})

    assert set(leaked) == {11}


def test_a_recycled_pid_is_not_waved_through() -> None:
    """Same pid, different start time, across a multi-hour gate."""
    current = {10: _facts(10, 900.0)}

    leaked = ORPHANS.started_during_run(current, {10: 100.0})

    assert set(leaked) == {10}, (
        "a pid recycled onto a fresh capsem process would otherwise pass as "
        "pre-existing -- precisely the leak this exists to catch"
    )


def test_a_process_outside_the_checkout_is_never_ours(tmp_path: Path) -> None:
    installed = _facts(12, 100.0, exe=str(Path.home() / ".capsem/bin/capsem-service"))

    assert not ORPHANS._from_this_tree(installed, ROOT), (
        "the user's installed daemon is not the gate's to reap"
    )


def test_a_process_from_this_checkout_is_ours() -> None:
    ours = _facts(13, 100.0, exe=str(ROOT / "cache/target/cargo/debug/capsem-service"))

    assert ORPHANS._from_this_tree(ours, ROOT)


# ---------------------------------------------------------------------------
# Functional: the check finds and reaps a real survivor
# ---------------------------------------------------------------------------


def _start_long_lived_capsem_fixture(binary: Path) -> subprocess.Popen:
    """Start a process whose Linux name remains the fixture's ``capsem-*`` name.

    This host's ``/bin/sleep`` is the uutils multicall launcher: copying it
    under a new basename makes it dispatch that basename and immediately fail
    with ``unknown program``. A copied Bash blocks in its ``read`` builtin, so
    it needs no adjacent runtime and leaves no child behind when reaped. macOS
    keeps the signed sleep fixture already proven there.
    """
    if sys.platform == "linux":
        shutil.copy("/bin/bash", binary)
        argv = [str(binary), "-c", "read -r _"]
    else:
        shutil.copy("/bin/sleep", binary)
        argv = [str(binary), "300"]
    # Copying strips the signature, and Apple Silicon kills an unsigned binary
    # at exec. Same ad-hoc signing every other test fixture uses; a no-op on
    # Linux.
    sign_binary(binary)
    # Keep stdin open so the Linux Bash remains blocked in its own builtin;
    # sleep ignores the pipe on macOS. The caller owns and closes it.
    return subprocess.Popen(argv, stdin=subprocess.PIPE)


@pytest.mark.skipif(not Path("/bin/sleep").exists(), reason="needs /bin/sleep")
def test_check_detects_and_reaps_a_real_orphan(tmp_path: Path) -> None:
    """End-to-end against a live process, scoped to a throwaway root.

    Scoped deliberately: sweeping the real checkout here would let this test
    SIGKILL a sibling xdist worker's live service.
    """
    fake_root = tmp_path / "checkout"
    (fake_root / "cache" / "target" / "cargo" / "debug").mkdir(parents=True)
    binary = fake_root / "cache" / "target" / "cargo" / "debug" / "capsem-fake-leak"

    baseline_file = tmp_path / "baseline.json"
    assert (
        ORPHANS.main(["baseline", "--baseline-file", str(baseline_file), "--root", str(fake_root)])
        == 0
    )
    assert json.loads(baseline_file.read_text()) == {}

    leaked = _start_long_lived_capsem_fixture(binary)
    try:
        deadline = time.time() + 10
        while time.time() < deadline:
            if ORPHANS.repo_capsem_processes(fake_root):
                break
            time.sleep(0.05)
        assert ORPHANS.repo_capsem_processes(fake_root), "fake orphan never became visible"

        status = ORPHANS.main(
            ["check", "--baseline-file", str(baseline_file), "--root", str(fake_root)]
        )

        assert status == 1, "a process that outlived the gate must fail the gate"
        assert leaked.poll() is not None, "the orphan must be reaped, not just reported"
    finally:
        if leaked.poll() is None:
            leaked.kill()
        leaked.wait()
        assert leaked.stdin is not None
        leaked.stdin.close()


def test_baseline_announces_leftovers_from_an_earlier_run(tmp_path: Path, capsys) -> None:
    """A process the baseline absorbs is one the check can never flag again.

    A run killed outright never reaches its trap, so its orphans are still
    there when the next gate takes its baseline. Recording them silently makes
    them permanently invisible -- which is how they reach five hours old.
    """
    fake_root = tmp_path / "checkout"
    (fake_root / "cache" / "target" / "cargo" / "debug").mkdir(parents=True)
    binary = fake_root / "cache" / "target" / "cargo" / "debug" / "capsem-fake-leftover"

    leftover = _start_long_lived_capsem_fixture(binary)
    try:
        deadline = time.time() + 10
        while time.time() < deadline and not ORPHANS.repo_capsem_processes(fake_root):
            time.sleep(0.05)

        baseline_file = tmp_path / "baseline.json"
        assert (
            ORPHANS.main(
                ["baseline", "--baseline-file", str(baseline_file), "--root", str(fake_root)]
            )
            == 0
        )

        err = capsys.readouterr().err
        assert "already running before the gate started" in err
        assert str(leftover.pid) in err

        # Still not blamed on this run: the point is visibility, not a new
        # failure mode for a developer's own dev daemon.
        assert (
            ORPHANS.main(["check", "--baseline-file", str(baseline_file), "--root", str(fake_root)])
            == 0
        )
        assert leftover.poll() is None, "a pre-existing process must not be reaped"
    finally:
        leftover.kill()
        leftover.wait()
        assert leftover.stdin is not None
        leftover.stdin.close()


def test_check_refuses_to_guess_without_a_baseline(tmp_path: Path, capsys) -> None:
    status = ORPHANS.main(
        ["check", "--baseline-file", str(tmp_path / "absent.json"), "--root", str(tmp_path)]
    )

    assert status == 2, "a missing baseline is a wiring bug, not a clean run"
    assert "no process baseline" in capsys.readouterr().err


def test_clean_run_passes(tmp_path: Path) -> None:
    baseline_file = tmp_path / "baseline.json"
    ORPHANS.main(["baseline", "--baseline-file", str(baseline_file), "--root", str(tmp_path)])

    assert (
        ORPHANS.main(["check", "--baseline-file", str(baseline_file), "--root", str(tmp_path)]) == 0
    )
