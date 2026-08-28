"""Ctrl-C has to mean stop, without meaning stop mid-write.

`planrunner` owned its `ThreadPoolExecutor` through a `with` block, and that
context manager's exit joins every running future. An interrupt fifty
milliseconds into a 750ms action therefore returned after 756ms; against a real
copy or image assembly, the operator presses Ctrl-C and watches nothing happen
for minutes.

Returning immediately would be worse, and that is why this is cooperative
rather than a `shutdown(wait=False)`. The machine lock, the workspace and the
service are released on the way out, and releasing them under a worker still
writing turns an interrupt into corruption.
"""

from __future__ import annotations

import os
import sys
import threading
import time
from pathlib import Path

import pytest
from capsem_builder.gate import cancellation
from capsem_builder.gate import config as gate_config
from capsem_builder.gate.actions import Action
from capsem_builder.gate.context import Context
from capsem_builder.gate.execution import step
from capsem_builder.gate.plan import Plan
from capsem_builder.gate.planrunner import execute
from capsem_builder.gate.proc import Runner

PROJECT_ROOT = Path(__file__).resolve().parents[1]
CONFIG = gate_config.load(PROJECT_ROOT)


class _Watching(Action, name="watching"):
    """A long action that asks, at a boundary, whether it is still wanted."""

    def __init__(self, *, started: threading.Event, ticks: int = 200) -> None:
        self.started = started
        self.ticks = ticks
        self.completed = False

    def render(self) -> str:
        return "watch for cancellation"

    def perform(self, context: Context) -> None:
        del context
        self.started.set()
        for _ in range(self.ticks):
            cancellation.check("the watching action")
            time.sleep(0.01)
        self.completed = True


class _Oblivious(Action, name="oblivious"):
    """One that never asks. Somebody has to be the stubborn worker."""

    def __init__(self, *, started: threading.Event, seconds: float) -> None:
        self.started = started
        self.seconds = seconds

    def render(self) -> str:
        return "ignore cancellation"

    def perform(self, context: Context) -> None:
        del context
        self.started.set()
        time.sleep(self.seconds)


class _Ran(Action, name="ran"):
    def __init__(self) -> None:
        self.ran = False

    def render(self) -> str:
        return "record that this ran"

    def perform(self, context: Context) -> None:
        del context
        self.ran = True


class _Foreground(Action, name="foreground-cancellation-fixture"):
    def __init__(self, pids: Path) -> None:
        self.pids = pids

    def render(self) -> str:
        return "run a foreground process tree"

    def perform(self, context: Context) -> None:
        child = "import signal; signal.alarm(5); signal.pause()"
        helper = (
            "import os,signal,subprocess,sys; signal.alarm(5); "
            f"child=subprocess.Popen([sys.executable,'-c',{child!r}]); "
            "open(sys.argv[1],'w').write(f'{os.getpid()} {child.pid}'); signal.pause()"
        )
        context.runner.run(
            (sys.executable, "-c", helper, str(self.pids)),
            log=self.pids.with_suffix(".log"),
        )


def _context(config=CONFIG) -> Context:
    from helpers.gate import RecordingJournal, RecordingRunner

    return Context(RecordingRunner(PROJECT_ROOT), config, journal=RecordingJournal())


def _interrupt_when(flag: threading.Event) -> threading.Thread:
    """Send this process a real SIGINT once the run is underway.

    Not `_thread.interrupt_main()`. That sets a flag CPython checks between
    bytecodes, and the main thread here is blocked in `concurrent.futures.wait`
    -- so it would be noticed only once the future it is waiting on finished,
    which is the exact behaviour under test. A signal interrupts the wait
    itself, which is what pressing Ctrl-C does.
    """
    import os
    import signal

    def fire() -> None:
        flag.wait(timeout=5)
        time.sleep(0.05)
        os.kill(os.getpid(), signal.SIGINT)

    thread = threading.Thread(target=fire, daemon=True)
    thread.start()
    return thread


def _pid_alive(pid: int) -> bool:
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    return True


def test_an_interrupt_stops_a_long_action_at_its_next_boundary() -> None:
    started = threading.Event()
    watching = _Watching(started=started)
    plan = Plan("interrupted")
    plan.add(step("long", watching))

    _interrupt_when(started)
    began = time.monotonic()
    with pytest.raises(KeyboardInterrupt):
        execute(plan, _context())
    elapsed = time.monotonic() - began

    assert not watching.completed, "the action ran to completion anyway"
    assert elapsed < 1.0, f"waited {elapsed:.2f}s for a two-second action"


def test_a_step_that_had_not_started_never_runs() -> None:
    """Cancelling the pending future is the difference between stopping and
    stopping *eventually*."""
    started = threading.Event()
    watching = _Watching(started=started)
    later = _Ran()
    plan = Plan("interrupted")
    first = plan.add(step("long", watching))
    plan.add(step("after", later), after=(first,))

    _interrupt_when(started)
    with pytest.raises(KeyboardInterrupt):
        execute(plan, _context())

    assert not later.ran


def test_a_step_asleep_on_a_resource_is_woken_to_notice() -> None:
    """Otherwise it waits for a holder that is also stopping.

    Nobody would notify it, so the bounded join would expire on a thread that
    was never going to move.
    """
    started = threading.Event()
    docker = CONFIG.exclusive("docker_daemon")
    holder = _Watching(started=started)
    waiter = _Ran()

    plan = Plan("contended")
    plan.add(step("holder", holder, contends=(docker,)))
    plan.add(step("waiter", waiter, contends=(docker,)))

    _interrupt_when(started)
    began = time.monotonic()
    with pytest.raises(KeyboardInterrupt):
        execute(plan, _context())
    elapsed = time.monotonic() - began

    assert elapsed < 2.0, f"the contended step held the run for {elapsed:.2f}s"


def test_ctrl_c_reaps_foreground_descendants_before_return(tmp_path: Path) -> None:
    pids = tmp_path / "owned-pids"
    plan = Plan("foreground")
    plan.add(step("foreground", _Foreground(pids)))

    # The interrupt trigger needs the child boundary, not merely submission.
    started = threading.Event()

    def notice_child() -> None:
        deadline = time.monotonic() + 2
        while not pids.is_file() and time.monotonic() < deadline:
            time.sleep(0.01)
        started.set()

    watcher = threading.Thread(target=notice_child, daemon=True)
    watcher.start()
    _interrupt_when(started)
    config = CONFIG.model_copy(
        update={
            "execution": CONFIG.execution.model_copy(update={"cancellation_grace_seconds": 0.5})
        }
    )
    context = Context(Runner(PROJECT_ROOT), config)
    with pytest.raises(KeyboardInterrupt):
        execute(plan, context)
    watcher.join(timeout=2)

    owned = [int(raw) for raw in pids.read_text().split()]
    assert all(not _pid_alive(pid) for pid in owned), owned


def test_a_worker_that_refuses_to_stop_is_named() -> None:
    """The bound is what stops an interrupt being indefinite; saying who is
    still going is what stops it being a mystery."""
    from helpers.gate import RecordingJournal

    started = threading.Event()
    plan = Plan("stubborn")
    plan.add(step("oblivious", _Oblivious(started=started, seconds=1.5)))

    execution = CONFIG.execution.model_copy(update={"cancellation_grace_seconds": 0.2})
    context = _context(CONFIG.model_copy(update={"execution": execution}))
    _interrupt_when(started)
    with pytest.raises(KeyboardInterrupt):
        execute(plan, context)

    journal = context.journal
    assert isinstance(journal, RecordingJournal)
    assert any("oblivious" in note for note in journal.notes), journal.notes


def test_cancellation_is_off_outside_a_run() -> None:
    """`check` must be free and silent when nothing armed it -- the primitives
    call it from ordinary code paths too."""
    assert not cancellation.stopped()
    cancellation.check("nothing")


def test_a_copy_stops_between_files(tmp_path: Path) -> None:
    """Between files, never inside one: a half-written file is worse than a
    half-copied tree, which the caller's cleanup can see and redo."""
    from capsem_builder.gate.filesystem import copy_tree

    source = tmp_path / "source"
    source.mkdir()
    for index in range(50):
        (source / f"{index:03d}.bin").write_bytes(b"x" * 1024)

    with cancellation.cancellable() as flag:
        flag.set()
        with pytest.raises(cancellation.Cancelled):
            copy_tree(source, tmp_path / "target")

    copied = list((tmp_path / "target").glob("*.bin"))
    assert len(copied) < 50
    for path in copied:
        assert path.stat().st_size == 1024, "a file was left half written"
