"""One gate per machine, and the three ways that used to go wrong silently.

A second `just test-clean` on the same machine is not a queueing inconvenience. The
first thing a run does is remove `$CAPSEM_HOME` (justfile:502) and stop the
service in it, so two runs means one deletes the other's home mid-flight and
both report failures that belong to neither.

`scripts/lib/exec_lock.sh` got this right, and every subtlety in it was a
comment. The lockfile had to sit outside the tree about to be wiped. The daemon
had to be launched with the lock fd closed (`3>&-`, justfile:152) or it would
hold the lock after the gate exited. And a dead holder had to release without
anyone cleaning up after it. None of the three was checked by anything.
"""

from __future__ import annotations

import ast
import contextlib
import json
import os
import signal
import subprocess
import sys
import textwrap
import time
from pathlib import Path

import pytest
from capsem_builder.gate import config as gate_config
from capsem_builder.gate.errors import GateError
from capsem_builder.gate.locks import ExclusiveLock
from capsem_builder.gate.lockschema import LockConfig

PROJECT_ROOT = Path(__file__).resolve().parents[1]
CONFIG = gate_config.load(PROJECT_ROOT)


def _settings(tmp_path: Path, **overrides) -> LockConfig:
    """Real policy, with the waits shortened so a test is not a stopwatch."""
    return LockConfig(
        **{
            "path": str(tmp_path / "gate.lock"),
            "holder_record": str(tmp_path / "gate.holder"),
            "report_after_seconds": 0.01,
            "wait_timeout_seconds": 0.2,
            "poll_interval_seconds": 0.01,
            "run_marker": "CAPSEM_GATE_RUN",
            **overrides,
        }
    )


HOLD_IT = """
    import fcntl, os, sys, time
    fd = os.open(sys.argv[1], os.O_RDWR | os.O_CREAT, 0o644)
    fcntl.flock(fd, fcntl.LOCK_EX)
    print("held", flush=True)
    time.sleep(float(sys.argv[2]))
"""


def _holding(lockfile: str, seconds: float) -> subprocess.Popen[str]:
    """A separate process that has the lock by the time this returns."""
    holder = subprocess.Popen(
        [sys.executable, "-c", textwrap.dedent(HOLD_IT), lockfile, str(seconds)],
        stdout=subprocess.PIPE,
        text=True,
    )
    assert holder.stdout is not None
    assert holder.stdout.readline().strip() == "held"
    return holder


def _in_a_subprocess(script: str, *args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, "-c", textwrap.dedent(script), *args],
        capture_output=True,
        text=True,
        check=False,
        cwd=PROJECT_ROOT,
    )


TAKE_IT = """
    import fcntl, os, sys
    fd = os.open(sys.argv[1], os.O_RDWR | os.O_CREAT, 0o644)
    try:
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        print("busy")
    else:
        print("free")
"""


def _is_free(lockfile: str) -> bool:
    """Whether another process could take this lock right now."""
    return _in_a_subprocess(TAKE_IT, lockfile).stdout.strip() == "free"


# ---------------------------------------------------------------------------
# Exclusion
# ---------------------------------------------------------------------------


def test_a_second_holder_is_refused_while_the_first_holds(tmp_path: Path) -> None:
    """The whole point: the second run must not start wiping the first's home."""
    settings = _settings(tmp_path)
    lock = ExclusiveLock(settings, purpose="just test-clean")
    lock.acquire()

    try:
        assert not _is_free(settings.path)
    finally:
        lock.release()


def test_the_lock_is_free_again_once_released(tmp_path: Path) -> None:
    settings = _settings(tmp_path)
    lock = ExclusiveLock(settings, purpose="just test-clean")
    lock.acquire()
    lock.release()

    assert _is_free(settings.path)


def test_the_kernel_releases_the_lock_when_the_holder_dies(tmp_path: Path) -> None:
    """`flock`, not a pidfile, for exactly this.

    A pidfile needs a staleness heuristic -- is this pid alive, is it still the
    same process, was the file left by a crash -- and every one of those is a
    way to either wedge the machine or let two runs start. The kernel drops the
    lock when the fd closes, including when the process is killed.
    """
    settings = _settings(tmp_path)
    with _holding(settings.path, seconds=60) as holder:
        assert not _is_free(settings.path)
        holder.kill()
        holder.wait(timeout=10)

    assert _is_free(settings.path), "a killed gate must not wedge the machine"


def test_a_launched_daemon_does_not_inherit_the_lock(tmp_path: Path) -> None:
    """The `3>&-` at justfile:152, obtained structurally instead of by memory.

    `_ensure-service` launches `capsem-service` under `nohup` while the gate
    holds the exec lock. If the daemon inherits the fd, it holds the lock for
    as long as it lives, so the next run blocks on a gate that finished hours
    ago -- a hang nobody attributes to a file descriptor.

    The gate here dies without releasing, which is the case that matters. On
    the ordinary path `release` unlocks explicitly, and an unlock frees the
    whole open file description including any copy a child holds -- so an
    inherited fd would be invisible. Only a killed gate exposes it, and a
    killed gate is exactly when a wedged machine is least welcome.

    The daemon is launched with `close_fds=False` because that is what a shell
    does, and a shell is what launches it. `subprocess` closes everything by
    default, which would make this pass no matter what the lock did.
    """
    settings = _settings(tmp_path)
    gate = subprocess.Popen(
        [
            sys.executable,
            "-c",
            textwrap.dedent("""
                import json, os, subprocess, sys
                sys.path.insert(0, "src")
                from capsem_builder.gate.lockschema import LockConfig
                from capsem_builder.gate.locks import ExclusiveLock

                ExclusiveLock(LockConfig(**json.loads(sys.argv[1])),
                              purpose="just test-clean").acquire()
                daemon = subprocess.Popen(
                    [sys.executable, "-c", "import time; time.sleep(60)"],
                    close_fds=False,
                )
                print(daemon.pid, flush=True)
                os._exit(1)          # killed mid-run: no release, no teardown
            """),
            settings.model_dump_json(),
        ],
        stdout=subprocess.PIPE,
        text=True,
        cwd=PROJECT_ROOT,
    )
    with gate:
        assert gate.stdout is not None
        daemon_pid = int(gate.stdout.readline().strip())
        gate.wait(timeout=10)

    try:
        assert _is_free(settings.path), (
            "the daemon inherited the lock fd and still holds it after the "
            "gate died; the machine is wedged until it is found and stopped"
        )
    finally:
        # By pid, never by name: this test's own guard forbids the latter, and
        # the reason is that a name matches processes nobody meant to touch.
        with contextlib.suppress(ProcessLookupError):
            os.kill(daemon_pid, signal.SIGKILL)


def test_the_lock_fd_is_explicitly_marked_uninheritable(tmp_path: Path) -> None:
    """Python's default since PEP 446, stated in the code so that adding
    `pass_fds` to a launch is a visible decision rather than a quiet one."""
    settings = _settings(tmp_path)
    lock = ExclusiveLock(settings, purpose="just test-clean")
    lock.acquire()

    try:
        assert os.get_inheritable(lock.fileno) is False
    finally:
        lock.release()


# ---------------------------------------------------------------------------
# Telling the operator what is going on
# ---------------------------------------------------------------------------


def test_contention_names_the_holder_rather_than_hanging(tmp_path: Path) -> None:
    """Someone who left `just test-clean` running in another terminal deserves to be
    told that, not to watch a cursor for two hours."""
    settings = _settings(tmp_path)
    first = ExclusiveLock(settings, purpose="just release-binaries nightly")
    first.acquire()

    try:
        with pytest.raises(GateError) as failure:
            ExclusiveLock(settings, purpose="just test-clean").acquire()

        message = str(failure.value)
        assert "release-binaries" in message, "say what is holding it"
        assert str(os.getpid()) in message, "and which process, so it can be checked"
    finally:
        first.release()


def test_the_holder_record_says_who_when_and_where(tmp_path: Path) -> None:
    settings = _settings(tmp_path)
    lock = ExclusiveLock(settings, purpose="just test-clean")

    lock.acquire()
    try:
        record = json.loads(Path(settings.holder_record).read_text())
    finally:
        lock.release()

    assert record["pid"] == os.getpid()
    assert record["purpose"] == "just test-clean"
    assert record["started"] <= time.time()
    assert record["host"]


def test_the_holder_record_is_cleared_on_release(tmp_path: Path) -> None:
    """A stale record would name a process that finished, sending whoever
    reads it to inspect the wrong pid."""
    settings = _settings(tmp_path)
    lock = ExclusiveLock(settings, purpose="just test-clean")
    lock.acquire()
    lock.release()

    assert not Path(settings.holder_record).exists()


def test_a_missing_holder_record_still_reports_useful_contention(
    tmp_path: Path,
) -> None:
    """The record is written after the lock is taken, so there is a window
    where it is absent -- and a crash can leave it absent for good. Contention
    must still say something rather than raising about the record."""
    settings = _settings(tmp_path)
    first = ExclusiveLock(settings, purpose="just test-clean")
    first.acquire()
    Path(settings.holder_record).unlink()

    try:
        with pytest.raises(GateError, match="another gate"):
            ExclusiveLock(settings, purpose="just smoke").acquire()
    finally:
        first.release()


def test_a_corrupt_holder_record_does_not_mask_the_contention(
    tmp_path: Path,
) -> None:
    """Half a JSON object is what a killed writer leaves behind."""
    settings = _settings(tmp_path)
    first = ExclusiveLock(settings, purpose="just test-clean")
    first.acquire()
    Path(settings.holder_record).write_text('{"pid": 4')

    try:
        with pytest.raises(GateError, match="another gate"):
            ExclusiveLock(settings, purpose="just smoke").acquire()
    finally:
        first.release()


# ---------------------------------------------------------------------------
# Waiting
# ---------------------------------------------------------------------------


def test_the_lock_waits_for_its_timeout_before_giving_up(tmp_path: Path) -> None:
    """Queueing behind another run is normal; failing instantly would make
    two developers on one machine unable to work."""
    settings = _settings(tmp_path, wait_timeout_seconds=0.3)
    first = ExclusiveLock(settings, purpose="just test-clean")
    first.acquire()

    started = time.monotonic()
    try:
        with pytest.raises(GateError):
            ExclusiveLock(settings, purpose="just smoke").acquire()
    finally:
        first.release()

    assert time.monotonic() - started >= 0.3, "it must actually wait"


def test_a_waiting_gate_takes_the_lock_when_it_is_freed(tmp_path: Path) -> None:
    """Otherwise queued work fails rather than running."""
    settings = _settings(tmp_path, wait_timeout_seconds=10)
    with _holding(settings.path, seconds=0.4) as holder:
        waiting = ExclusiveLock(settings, purpose="just test-clean")
        try:
            waiting.acquire()
        finally:
            waiting.release()
        holder.wait(timeout=10)


# ---------------------------------------------------------------------------
# Placement
# ---------------------------------------------------------------------------


def test_the_checked_in_lock_sits_outside_every_tree_the_gate_wipes() -> None:
    """The run takes the lock and *then* removes CAPSEM_HOME.

    A lockfile inside that tree is deleted while held, so the next run creates
    a fresh inode, takes a lock on it, and starts beside the first -- both of
    them convinced they were alone.
    """
    lock = Path(CONFIG.locks.gate.path)
    holder = Path(CONFIG.locks.gate.holder_record)

    for reclaimable in CONFIG.disk.reclaimable:
        tree = Path(reclaimable)
        assert tree not in lock.parents, f"{lock} sits inside {tree}"
        assert tree not in holder.parents, f"{holder} sits inside {tree}"


def test_the_lock_is_a_resource_so_teardown_is_not_optional() -> None:
    """Held through `held(...)` with everything else, released in reverse --
    so an interrupted gate drops it rather than wedging the machine."""
    from capsem_builder.gate.lifecycle import Resource

    assert issubclass(ExclusiveLock, Resource)


def test_asking_for_the_fd_of_an_unheld_lock_says_so(tmp_path: Path) -> None:
    """Rather than returning a stale descriptor from a previous acquisition."""
    lock = ExclusiveLock(_settings(tmp_path), purpose="just test-clean")

    with pytest.raises(GateError, match="not held"):
        _ = lock.fileno


def test_releasing_a_lock_never_taken_is_harmless(tmp_path: Path) -> None:
    """`held` releases only what it acquired, but teardown runs against
    whatever state a failure left, so this must not raise."""
    ExclusiveLock(_settings(tmp_path), purpose="just test-clean").release()


def test_the_gate_lock_is_built_from_the_checked_in_policy() -> None:
    """One lockfile, named once, so two callers cannot take different ones."""
    lock = ExclusiveLock.for_gate(CONFIG, purpose="just test-clean")

    assert lock.path == Path(CONFIG.locks.gate.path).expanduser()


def test_worktrees_contend_on_one_user_scoped_machine_lock(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """A checkout-relative lock lets two worktrees wipe one shared machine."""
    home = tmp_path / "home"
    monkeypatch.setenv("HOME", str(home))
    policy = CONFIG.locks.gate.model_copy(
        update={
            "report_after_seconds": 0.01,
            "wait_timeout_seconds": 0.05,
            "poll_interval_seconds": 0.01,
        }
    )
    locks = CONFIG.locks.model_copy(update={"gate": policy})
    first_config = CONFIG.model_copy(update={"root": tmp_path / "worktree-a", "locks": locks})
    second_config = CONFIG.model_copy(update={"root": tmp_path / "worktree-b", "locks": locks})
    first = ExclusiveLock.for_gate(first_config, purpose="worktree-a")
    second = ExclusiveLock.for_gate(second_config, purpose="worktree-b")

    assert first.path == second.path
    assert first.path.is_relative_to(home)
    first.acquire()
    try:
        with pytest.raises(GateError, match="worktree-a"):
            second.acquire()
    finally:
        first.release()


def test_contention_is_never_resolved_by_signalling_the_holder() -> None:
    """Stopping a gate that holds the lock is the operator's business.

    A lock that clears itself by killing whoever has it is not a lock, and the
    process it would reach for is as likely to be a developer's own daemon.
    Checked as code rather than as text, so the prose above may say "killed".
    """
    tree = ast.parse(
        (PROJECT_ROOT / "build_system" / "builder" / "gate" / "locks.py").read_text(encoding="utf-8")
    )
    calls = {
        node.func.attr
        for node in ast.walk(tree)
        if isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute)
    }

    assert "kill" not in calls
    assert "killpg" not in calls
