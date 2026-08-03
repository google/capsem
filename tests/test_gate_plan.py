"""Order comes from the graph, not from the order somebody typed the steps.

The install gate's defect was an ordering defect: a manifest URL consumed
before anything staged the file it pointed at. A hand-written list can express
that mistake -- the fix was to move two lines, and the test that caught it had
transcribed the wrong order and would have failed the fix.

A declared edge cannot express it. `install` is `after=(stage,)`, and no
arrangement of the source lines changes what runs first. What the sort makes
simultaneously ready is independent *by construction*, so parallelism stops
being a human judgement about which jobs are safe beside each other.
"""

from __future__ import annotations

import threading
import time
from pathlib import Path

import pytest
from helpers.gate import RecordingJournal, RecordingRunner

from capsem.gate import config as gate_config
from capsem.gate.actions import Action, Run
from capsem.gate.context import Context
from capsem.gate.errors import GateError
from capsem.gate.execution import step
from capsem.gate.plan import Plan

PROJECT_ROOT = Path(__file__).resolve().parents[1]
CONFIG = gate_config.load(PROJECT_ROOT)

VZ = CONFIG.exclusive("apple_vz")
SERVICE = CONFIG.exclusive("host_service")

#: Long enough that two truly concurrent steps overlap even on a loaded
#: machine, short enough that the suite stays fast.
HOLD = 0.05


@pytest.fixture
def context() -> Context:
    return Context(RecordingRunner(PROJECT_ROOT), CONFIG)


class Tracked(Action, name="tracked"):
    """Records the window it was inside its own body, so overlap is visible."""

    def __init__(self, label: str, spans: list, *, fail: bool = False) -> None:
        self._label = label
        self._spans = spans
        self._fail = fail
        self._lock = threading.Lock()

    def render(self) -> str:
        return f"tracked {self._label}"

    def perform(self, context: Context) -> None:
        started = time.monotonic()
        time.sleep(HOLD)
        with self._lock:
            self._spans.append((self._label, started, time.monotonic()))
        if self._fail:
            raise GateError(f"{self._label} broke")


def _tracked(label: str, spans: list, *, fail: bool = False, **kwargs):
    return step(label, Tracked(label, spans, fail=fail), **kwargs)


def _appending(label: str, ran: list, **kwargs):
    class Append(Action, name="append"):
        def render(self) -> str:
            return f"append {label}"

        def perform(self, context: Context) -> None:
            ran.append(label)

    return step(label, Append(), **kwargs)


def _raising(label: str, **kwargs):
    class Raise(Action, name="raise"):
        def render(self) -> str:
            return f"fail {label}"

        def perform(self, context: Context) -> None:
            raise GateError(f"{label} broke")

    return step(label, Raise(), **kwargs)


def _noop():
    class Noop(Action, name="noop"):
        def render(self) -> str:
            return "noop"

        def perform(self, context: Context) -> None: ...

    return (Noop(),)


def _overlap(spans: list, first: str, second: str) -> bool:
    a = next(s for s in spans if s[0] == first)
    b = next(s for s in spans if s[0] == second)
    return a[1] < b[2] and b[1] < a[2]


# ---------------------------------------------------------------------------
# Order is derived
# ---------------------------------------------------------------------------


def test_a_step_runs_after_everything_it_declared(context: Context) -> None:
    ran: list[str] = []
    plan = Plan("example")
    a = plan.add(_appending("a", ran))
    b = plan.add(_appending("b", ran))
    plan.add(_appending("c", ran), after=(a, b))

    plan.run(context)

    assert ran.index("c") > ran.index("a")
    assert ran.index("c") > ran.index("b")


def test_the_order_does_not_depend_on_declaration_order(context: Context) -> None:
    """The property a hand-written list cannot have.

    `_gate-install` handed the installer a manifest URL before anything wrote
    the manifest, and the fix was to move two lines. Here the lines are in the
    wrong order on purpose and the run is still correct.
    """
    ran: list[str] = []
    plan = Plan("reversed")
    consume = plan.add(_appending("consume", ran))
    stage = plan.add(_appending("stage", ran))
    plan.edge(before=stage, after=consume)

    plan.run(context)

    assert ran == ["stage", "consume"]


def test_a_cycle_fails_before_anything_runs(context: Context) -> None:
    """Named at plan time, not discovered forty minutes in."""
    plan = Plan("cyclic")
    a = plan.add(_appending("a", []))
    b = plan.add(_appending("b", []), after=(a,))
    plan.edge(before=b, after=a)

    with pytest.raises(GateError, match="cycle"):
        plan.order()


def test_a_cycle_names_the_steps_involved(context: Context) -> None:
    plan = Plan("cyclic")
    a = plan.add(_appending("stage", []))
    b = plan.add(_appending("install", []), after=(a,))
    plan.edge(before=b, after=a)

    with pytest.raises(GateError) as failure:
        plan.run(context)

    assert "stage" in str(failure.value)
    assert "install" in str(failure.value)


def test_an_edge_to_an_unregistered_step_is_refused() -> None:
    """Otherwise the edge is silently dropped and the order silently wrong."""
    plan = Plan("example")
    registered = plan.add(_appending("a", []))
    stranger = _appending("elsewhere", [])

    with pytest.raises(GateError, match="elsewhere"):
        plan.edge(before=stranger, after=registered)


# ---------------------------------------------------------------------------
# Concurrency is derived too
# ---------------------------------------------------------------------------


def test_independent_steps_run_at_once(context: Context) -> None:
    """Parallelism comes from the graph, not from a human deciding which jobs
    are safe to put behind an `&`."""
    spans: list = []
    plan = Plan("wide")
    plan.add(_tracked("a", spans))
    plan.add(_tracked("b", spans))

    plan.run(context)

    assert _overlap(spans, "a", "b")


def test_a_dependent_step_waits(context: Context) -> None:
    spans: list = []
    plan = Plan("narrow")
    first = plan.add(_tracked("first", spans))
    plan.add(_tracked("second", spans), after=(first,))

    plan.run(context)

    assert not _overlap(spans, "first", "second")


# ---------------------------------------------------------------------------
# Contention
# ---------------------------------------------------------------------------


def test_two_claimants_of_one_exclusive_never_overlap(context: Context) -> None:
    """The whole reason exclusives exist: the graph says these are independent,
    and they still must not share the machine."""
    spans: list = []
    plan = Plan("bench")
    plan.add(_tracked("one", spans, contends=(VZ,)))
    plan.add(_tracked("two", spans, contends=(VZ,)))

    plan.run(context)

    assert not _overlap(spans, "one", "two")


def test_claimants_of_different_exclusives_still_overlap(context: Context) -> None:
    """Otherwise "serial" quietly means "serial with everything", and the
    exclusive stops being a statement about one resource."""
    spans: list = []
    plan = Plan("mixed")
    plan.add(_tracked("vz", spans, contends=(VZ,)))
    plan.add(_tracked("svc", spans, contends=(SERVICE,)))

    plan.run(context)

    assert _overlap(spans, "vz", "svc")


def test_a_failing_claimant_releases_its_exclusive(context: Context) -> None:
    """A lock still held after a failure deadlocks every later claimant, which
    turns one broken step into a gate that never returns."""
    spans: list = []
    plan = Plan("bench")
    plan.add(_tracked("one", spans, fail=True, contends=(VZ,)))
    plan.add(_tracked("two", spans, contends=(VZ,)))

    with pytest.raises(GateError):
        plan.run(context)

    assert {span[0] for span in spans} == {"one", "two"}


def test_a_step_claiming_two_exclusives_takes_them_in_one_order(
    context: Context,
) -> None:
    """Two steps claiming the same pair in opposite orders is a deadlock. The
    sort makes that unrepresentable rather than merely unlikely."""
    spans: list = []
    plan = Plan("both")
    plan.add(_tracked("a", spans, contends=(VZ, SERVICE)))
    plan.add(_tracked("b", spans, contends=(SERVICE, VZ)))

    plan.run(context)

    assert not _overlap(spans, "a", "b")


# ---------------------------------------------------------------------------
# Failure
# ---------------------------------------------------------------------------


def test_independent_failures_are_all_reported(context: Context) -> None:
    """Stopping at the first turns one push into three discoveries, which is
    how a gate takes three rounds to go green."""
    plan = Plan("audits")
    plan.add(_raising("cargo-audit"))
    plan.add(_appending("pnpm-audit", []))
    plan.add(_raising("python-audit"))

    with pytest.raises(GateError) as failure:
        plan.run(context)

    assert "cargo-audit" in str(failure.value)
    assert "python-audit" in str(failure.value)


def test_a_dependent_step_does_not_run_when_its_dependency_failed(
    context: Context,
) -> None:
    """Aggregation is right for independent work and wrong for a sequence:
    a step must not run against what the step before it failed to produce."""
    ran: list[str] = []
    plan = Plan("pipeline")
    first = plan.add(_raising("stage"))
    plan.add(_appending("install", ran), after=(first,))
    plan.add(_appending("elsewhere", ran))

    with pytest.raises(GateError, match="stage"):
        plan.run(context)

    assert "install" not in ran, "it depends on what failed"
    assert "elsewhere" in ran, "it does not, so it still runs"


def test_a_skipped_step_is_reported_as_skipped_not_passed(
    context: Context,
) -> None:
    """A step that never ran because its dependency broke is not a step that
    succeeded, and a report that conflates them hides the blast radius."""
    plan = Plan("pipeline")
    first = plan.add(_raising("stage"))
    plan.add(_appending("install", []), after=(first,))

    with pytest.raises(GateError) as failure:
        plan.run(context)

    assert "install" in str(failure.value)
    assert "skipped" in str(failure.value)


# ---------------------------------------------------------------------------
# What a step produces
# ---------------------------------------------------------------------------


def test_a_step_records_the_artifacts_it_declared(tmp_path: Path) -> None:
    """So a run log answers "which bytes did this build" after the tree that
    held them has been reclaimed."""

    journal = RecordingJournal()
    context = Context(RecordingRunner(PROJECT_ROOT), CONFIG, journal=journal)
    kernel = tmp_path / "vmlinuz"
    kernel.write_bytes(b"kernel")

    plan = Plan("build")
    plan.add(step("kernel", *_noop(), produces=(kernel,)))
    plan.run(context)

    assert [entry[0] for entry in journal.artifacts] == [kernel]


def test_a_failing_step_records_nothing_it_did_not_produce(tmp_path: Path) -> None:
    """Hashing after the actions, not before: a step that failed halfway has
    an output file, and recording it would claim a build that did not finish."""

    journal = RecordingJournal()
    context = Context(RecordingRunner(PROJECT_ROOT), CONFIG, journal=journal)
    half_built = tmp_path / "rootfs.erofs"
    half_built.write_bytes(b"partial")

    plan = Plan("build")
    plan.add(_raising("rootfs", produces=(half_built,)))

    with pytest.raises(GateError):
        plan.run(context)

    assert journal.artifacts == []


# ---------------------------------------------------------------------------
# Building the plan
# ---------------------------------------------------------------------------


def test_two_steps_cannot_share_a_label(context: Context) -> None:
    """A duplicate label makes the log ambiguous about which one was slow."""
    plan = Plan("example")
    plan.add(_appending("build", []))

    with pytest.raises(GateError, match="already has a step"):
        plan.add(_appending("build", []))


def test_the_outcomes_are_readable_after_a_run(context: Context) -> None:
    """The run log and the timing summary both read these."""
    plan = Plan("example")
    plan.add(_appending("a", []))

    plan.run(context)

    assert plan.outcomes["a"].status == "ok"
    assert plan.outcomes["a"].duration >= 0


def test_an_interrupt_is_not_recorded_as_a_step_that_failed(
    context: Context,
) -> None:
    """Ctrl-C is not a gate result.

    Recording it as a failed step would let the run finish and report on
    itself, which is the shell trap hazard in another form: `$?` inside EXIT
    read 0 on abort, so an interrupted run came back green.
    """

    class Interrupt(Action, name="interrupt"):
        def render(self) -> str:
            return "interrupt"

        def perform(self, context: Context) -> None:
            raise KeyboardInterrupt

    plan = Plan("aborted")
    plan.add(step("work", Interrupt()))

    with pytest.raises(KeyboardInterrupt):
        plan.run(context)


# ---------------------------------------------------------------------------
# The dry run
# ---------------------------------------------------------------------------


def test_a_dry_run_executes_nothing(context: Context) -> None:
    """The question "what would this do" must not cost forty minutes."""
    ran: list[str] = []
    plan = Plan("example")
    plan.add(step("build", Run(["cargo", "build", "--release"])))
    plan.add(_appending("other", ran))

    plan.describe()

    assert ran == []
    assert context.runner.commands == []


def test_a_dry_run_shows_the_argv_a_step_would_invoke(context: Context) -> None:
    """A plan whose steps are opaque can only print their names, which is a
    summary rather than something you can check before spending on it."""
    plan = Plan("example")
    plan.add(step("build", Run(["cargo", "build", "--release"])))

    rendering = plan.describe()

    assert "cargo build --release" in rendering


def test_a_dry_run_shows_the_waves_in_order(context: Context) -> None:
    plan = Plan("example")
    stage = plan.add(_appending("stage", []))
    plan.add(_appending("install", []), after=(stage,))

    rendering = plan.describe()

    assert rendering.index("stage") < rendering.index("install")


def test_a_dry_run_names_what_a_step_contends_for(context: Context) -> None:
    """Because a reader asking why two things are serialized deserves the
    reason, not the fact."""
    plan = Plan("bench")
    plan.add(_tracked("benchmark", [], contends=(VZ,)))

    rendering = plan.describe()

    assert VZ.name in rendering


def test_a_dry_run_says_it_executed_nothing(context: Context) -> None:
    """So a scrolled-back reader cannot mistake it for a run that happened."""
    plan = Plan("example")
    plan.add(_appending("a", []))

    assert "dry-run" in plan.describe()


# ---------------------------------------------------------------------------
# The graph, for a bug report or the docs
# ---------------------------------------------------------------------------


def test_the_graph_renders_one_node_per_step_and_one_edge_per_dependency(
    context: Context,
) -> None:
    plan = Plan("example")
    stage = plan.add(_appending("stage", []))
    plan.add(_appending("install", []), after=(stage,))

    graph = plan.mermaid()

    assert graph.splitlines()[0].startswith("graph")
    assert "stage" in graph and "install" in graph
    assert "-->" in graph


# ---------------------------------------------------------------------------
# Timing
# ---------------------------------------------------------------------------


def test_the_critical_path_is_the_longest_chain_not_the_slowest_step(
    context: Context,
) -> None:
    """The distinction that makes the number actionable.

    Shortening the slowest step does nothing if it runs beside something
    longer. Only the critical path is what the run's duration is made of.
    """
    spans: list = []
    plan = Plan("timed")
    quick = plan.add(_tracked("quick", spans))
    plan.add(_tracked("after-quick", spans), after=(quick,))
    plan.add(_tracked("lonely", spans))

    plan.run(context)

    assert [entry.label for entry in plan.critical_path()] == [
        "quick",
        "after-quick",
    ]


def test_the_critical_path_needs_a_run_behind_it() -> None:
    """Durations come from measurement, so asking before running is a mistake
    worth naming rather than an empty list."""
    plan = Plan("timed")
    plan.add(_appending("a", []))

    with pytest.raises(GateError, match="has not run"):
        plan.critical_path()


# ---------------------------------------------------------------------------
# Artifacts, and who owns them
# ---------------------------------------------------------------------------


def _producing(label: str, artifact: Path, **kwargs):
    return step(label, *_noop(), produces=(artifact,), **kwargs)


def test_two_steps_writing_one_path_must_share_an_exclusive(
    context: Context, tmp_path: Path
) -> None:
    """A lock around the mutation is not a lock around the artifact.

    A step can hold an exclusive while it builds, release it, and hand back
    "look at this path" -- and the next claimant overwrites that path before
    the consumer reads it. This is how four helpers came to lock `astro
    build`, release, then read a `dist/` the next build had already replaced.
    """
    shared = tmp_path / "dist"
    plan = Plan("two-owners")
    plan.add(_producing("build-a", shared))
    plan.add(_producing("build-b", shared))

    with pytest.raises(GateError, match="can be in flight together"):
        plan.run(context)


def test_sharing_an_exclusive_makes_two_producers_legal(context: Context, tmp_path: Path) -> None:
    """Serialized producers cannot overwrite each other mid-read."""
    shared = tmp_path / "dist"
    shared.write_text("built")
    plan = Plan("one-at-a-time")
    plan.add(_producing("build-a", shared, contends=(VZ,)))
    plan.add(_producing("build-b", shared, contends=(VZ,)))

    plan.run(context)


def test_one_producer_per_path_needs_no_exclusive(context: Context, tmp_path: Path) -> None:
    """The common case stays free of ceremony."""
    artifact = tmp_path / "vmlinuz"
    artifact.write_text("kernel")
    plan = Plan("single")
    plan.add(_producing("build", artifact))

    plan.run(context)


def test_the_conflict_is_reported_before_anything_runs(context: Context, tmp_path: Path) -> None:
    """By the time the overwrite happens the evidence is gone, so this is a
    plan-time error rather than something to notice afterwards."""
    ran: list[str] = []
    shared = tmp_path / "dist"
    plan = Plan("two-owners")
    plan.add(_producing("build-a", shared))
    plan.add(_producing("build-b", shared))
    plan.add(_appending("elsewhere", ran))

    with pytest.raises(GateError):
        plan.run(context)

    assert ran == [], "nothing may run before the plan is known to be sound"


# ---------------------------------------------------------------------------
# Infrastructure two fragments both need
# ---------------------------------------------------------------------------


def test_a_shared_step_is_added_once_and_depended_on_twice() -> None:
    """Two fragments needing the same groundwork is a diamond, not a clash.

    The Linux builder image is built by `install-image` and again by
    `cross-compile`. Composed into one plan they must not each add it -- and
    must not each build it either. `shared` makes the second caller a dependant
    of the first one's step, which is what the graph is for.
    """
    plan = Plan("example")
    first = plan.shared(_appending("host-image", []))
    second = plan.shared(_appending("host-image", []))

    assert first is second
    assert [step.label for step in plan.steps] == ["host-image"]


def test_a_shared_step_that_differs_is_still_a_collision() -> None:
    """Silently returning the first would run the wrong work for the second.

    Two steps sharing a name but not a definition is the ordinary duplicate
    bug, and `shared` must not become the place it hides.
    """
    plan = Plan("example")
    plan.shared(step("build", Run(["cargo", "build"])))

    with pytest.raises(GateError, match="two different steps"):
        plan.shared(step("build", Run(["cargo", "build", "--release"])))


def test_a_shared_step_can_be_depended_on_by_both_callers() -> None:
    """The point of the diamond: both dependants wait, the work happens once."""
    ran: list[str] = []
    plan = Plan("example")
    base = plan.shared(_appending("base", ran))
    plan.add(_appending("left", ran), after=(base,))
    plan.add(_appending("right", ran), after=(plan.shared(_appending("base", ran)),))

    assert plan.edges == (("base", "left"), ("base", "right"))
