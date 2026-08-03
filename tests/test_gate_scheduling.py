"""Who decides a step may start, and where its latency went.

Every dependency-ready step used to be submitted at once, and each worker then
blocked *inside* the resource lock until its claim was free. Two consequences,
both invisible from the graph:

  the pool was sized `max_workers=len(plan.steps)` -- eighty-one threads for
  the candidate plan -- because anything smaller could fill every worker with
  steps that were only waiting, and starve the one that could actually run

  a step's recorded duration was resource wait plus execution, so "the gate is
  slow" could not distinguish a step that took twenty minutes from a step that
  spent nineteen of them queued behind Docker

Claims are reserved by the coordinator before submission here. A worker only
ever holds a resource it already owns, the bound is a real bound, and the three
kinds of waiting are three numbers.
"""

from __future__ import annotations

import threading
import time
from pathlib import Path

import pytest

from capsem.gate import config as gate_config
from capsem.gate.actions import Action, Why
from capsem.gate.context import Context
from capsem.gate.execution import step
from capsem.gate.plan import Plan
from capsem.gate.planrunner import execute

PROJECT_ROOT = Path(__file__).resolve().parents[1]
CONFIG = gate_config.load(PROJECT_ROOT)

DOCKER = CONFIG.exclusive("docker_daemon")
DOCKER_SHARED = CONFIG.shared("docker_daemon")
VZ = CONFIG.exclusive("apple_vz")


class _Concurrent(Action, name="concurrent"):
    """Records how many of these were running at once."""

    def __init__(self, tracker: _Tracker, *, seconds: float = 0.05) -> None:
        self._tracker = tracker
        self._seconds = seconds

    def render(self) -> str:
        return "record concurrency"

    def perform(self, context: Context) -> None:
        del context
        with self._tracker.entered():
            time.sleep(self._seconds)


class _Tracker:
    def __init__(self) -> None:
        self._lock = threading.Lock()
        self.live = 0
        self.peak = 0
        self.overlapped = False

    def entered(self):
        from contextlib import contextmanager

        @contextmanager
        def scope():
            with self._lock:
                self.live += 1
                self.peak = max(self.peak, self.live)
                self.overlapped = self.overlapped or self.live > 1
            try:
                yield
            finally:
                with self._lock:
                    self.live -= 1

        return scope()


def _context() -> Context:
    from helpers.gate import RecordingJournal, RecordingRunner

    return Context(RecordingRunner(PROJECT_ROOT), CONFIG, journal=RecordingJournal())


# ---------------------------------------------------------------------------
# The predicate, shared by the validator, the scheduler and these tests
# ---------------------------------------------------------------------------


def test_two_shared_claims_on_one_resource_may_overlap() -> None:
    """Which is the whole reason `shared` exists: the asset lanes have to."""
    from capsem.gate.contention import can_overlap

    assert can_overlap(step("a", contends=(DOCKER_SHARED,)), step("b", contends=(DOCKER_SHARED,)))


@pytest.mark.parametrize(
    ("first", "second"),
    (
        ((DOCKER,), (DOCKER,)),
        ((DOCKER,), (DOCKER_SHARED,)),
        ((DOCKER_SHARED,), (DOCKER,)),
    ),
)
def test_an_exclusive_claim_excludes_both_kinds(first, second) -> None:
    from capsem.gate.contention import can_overlap

    assert not can_overlap(step("a", contends=first), step("b", contends=second))


def test_unrelated_resources_never_exclude_each_other() -> None:
    from capsem.gate.contention import can_overlap

    assert can_overlap(step("a", contends=(DOCKER,)), step("b", contends=(VZ,)))
    assert can_overlap(step("a"), step("b", contends=(DOCKER,)))


def test_the_validator_uses_the_same_predicate() -> None:
    """Two copies of this rule is how the guard came to agree with the bug."""
    source = (PROJECT_ROOT / "src/capsem/gate/planchecks.py").read_text(encoding="utf-8")

    assert "can_overlap" in source


# ---------------------------------------------------------------------------
# Claims are reserved before submission, so a worker never waits for one
# ---------------------------------------------------------------------------


def test_a_step_waiting_for_a_resource_occupies_no_worker() -> None:
    """The property the bound depends on.

    With workers blocking inside the lock, `max_parallel` steps that all want
    one exclusive would fill the pool and nothing else could run -- so the
    pool had to be as large as the plan.
    """
    tracker = _Tracker()
    plan = Plan("contended")
    # Three steps that must serialize, and one that is free to run beside any
    # of them. With a bound of two, the free step must not be stuck behind the
    # two contended ones that cannot both start.
    free = _Tracker()
    for index in range(3):
        plan.add(step(f"docker-{index}", _Concurrent(tracker), contends=(DOCKER,)))
    plan.add(step("free", _Concurrent(free, seconds=0.01)))

    execute(plan, _context(), max_parallel=2)

    assert tracker.peak == 1, "steps holding one exclusive ran together"
    assert free.peak == 1


def test_shared_holders_still_overlap() -> None:
    tracker = _Tracker()
    plan = Plan("lanes")
    for arch in ("arm64", "x86_64"):
        plan.add(step(f"lane-{arch}", _Concurrent(tracker, seconds=0.2), contends=(DOCKER_SHARED,)))

    execute(plan, _context(), max_parallel=4)

    assert tracker.overlapped, "the asset lanes must be able to run at once"


def test_the_parallel_bound_is_respected() -> None:
    tracker = _Tracker()
    plan = Plan("wide")
    for index in range(8):
        plan.add(step(f"free-{index}", _Concurrent(tracker, seconds=0.1)))

    execute(plan, _context(), max_parallel=3)

    assert tracker.peak <= 3, f"ran {tracker.peak} steps against a bound of 3"
    assert tracker.peak > 1, "a bound of three that never reaches two is not a bound"


def test_the_bound_comes_from_configuration() -> None:
    """Operational limits are configuration; scheduling semantics are code."""
    assert CONFIG.execution.max_parallel_steps >= 1


# ---------------------------------------------------------------------------
# Where the time went
# ---------------------------------------------------------------------------


def test_an_outcome_separates_dependency_wait_from_resource_wait(  # noqa: PLR0914
) -> None:
    """Three numbers, because they have three different fixes.

    A step recorded one duration covering resource wait *and* execution, so a
    twenty-minute step that spent nineteen queued behind Docker looked exactly
    like one doing twenty minutes of work.
    """
    plan = Plan("timed")
    tracker = _Tracker()
    holder = plan.add(step("holder", _Concurrent(tracker, seconds=0.3), contends=(DOCKER,)))
    plan.add(step("waiter", _Concurrent(tracker, seconds=0.01), contends=(DOCKER,)))
    plan.add(step("after", _Concurrent(_Tracker(), seconds=0.01)), after=(holder,))

    outcomes = execute(plan, _context(), max_parallel=4)

    waiter = outcomes["waiter"]
    after = outcomes["after"]

    assert waiter.resource_wait > 0.1, "the waiter queued behind the holder for free"
    assert waiter.execution < 0.1
    assert after.dependency_wait > 0.1, "a step behind a slow dependency waited for free"
    assert after.resource_wait < 0.1, "nothing contended with it"


def test_the_recorded_duration_is_still_the_whole_step() -> None:
    """Splitting it must not make the total disappear from the summary."""
    plan = Plan("timed")
    plan.add(step("only", _Concurrent(_Tracker(), seconds=0.05)))

    outcome = execute(plan, _context())["only"]

    assert outcome.duration >= outcome.execution > 0
