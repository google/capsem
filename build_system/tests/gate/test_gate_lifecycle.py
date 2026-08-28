"""Acquiring things the gate must give back, and giving them back in order.

Every one of these was a hand-written `finally` somewhere, where the contract
was the order of the lines and nothing checked it.

`assets.py` has to stop the service *before* deleting the run directory,
because stopping it is what flushes `serial.log` -- the file a boot failure is
argued from. `install.py` has to clear the manifest handoff *before* the
container goes, or the next install in the checkout inherits a request pointing
at a graph that no longer exists. Both are release-order rules, and a `finally`
block enforces order only by where its lines happen to sit.

`held` makes it structural: acquired in order, released in reverse, the way a
stack unwinds. `preserve` is the third phase and the one an ad-hoc `finally`
forgets -- it runs only on failure, and before release, because release is what
destroys the evidence.
"""

from __future__ import annotations

import pytest
from capsem_builder.gate.errors import GateError
from capsem_builder.gate.lifecycle import Resource, held


class Recorder(Resource, name="recorder"):
    """A resource that writes what happened to a log the test can read.

    `fail` names the phase that should break, so one class covers acquisition
    failures, teardown failures, and broken evidence collection.
    """

    def __init__(self, log: list[str], label: str, *, fail: str | None = None) -> None:
        self._log = log
        self._label = label
        self._fail = fail

    def acquire(self) -> None:
        if self._fail == "acquire":
            raise GateError(f"{self._label} could not be acquired")
        self._log.append(f"acquire {self._label}")

    def release(self) -> None:
        if self._fail == "release":
            raise GateError(f"{self._label} would not let go")
        self._log.append(f"release {self._label}")

    def preserve(self, error: BaseException) -> None:
        if self._fail == "preserve":
            raise GateError(f"{self._label} evidence collection broke")
        self._log.append(f"preserve {self._label}")


# ---------------------------------------------------------------------------
# Order
# ---------------------------------------------------------------------------


def test_resources_are_acquired_in_order_and_released_in_reverse() -> None:
    """The stack discipline that replaces getting the `finally` lines right."""
    log: list[str] = []

    with held(Recorder(log, "a"), Recorder(log, "b"), Recorder(log, "c")):
        log.append("body")

    assert log == [
        "acquire a",
        "acquire b",
        "acquire c",
        "body",
        "release c",
        "release b",
        "release a",
    ]


def test_a_resource_that_fails_to_acquire_is_never_released() -> None:
    """Releasing something never taken is how teardown corrupts good state.

    A half-built phase must leave nothing behind, and must not reach past the
    failure to tidy up something it never touched.
    """
    log: list[str] = []

    with (
        pytest.raises(GateError, match="could not be acquired"),
        held(
            Recorder(log, "a"),
            Recorder(log, "b", fail="acquire"),
            Recorder(log, "c"),
        ),
    ):
        log.append("body")

    assert "acquire c" not in log, "acquisition must stop at the failure"
    assert "body" not in log, "the body must not run"
    assert "release b" not in log, "b was never acquired, so b is not released"
    assert log[-1] == "release a", "a was acquired, so a is released"


# ---------------------------------------------------------------------------
# Evidence
# ---------------------------------------------------------------------------


def test_evidence_is_preserved_before_anything_is_released() -> None:
    """Release is what destroys the evidence, so preserve cannot come after."""
    log: list[str] = []

    with (
        pytest.raises(GateError, match="boom"),
        held(Recorder(log, "a"), Recorder(log, "b")),
    ):
        raise GateError("boom")

    assert log.index("preserve b") < log.index("release b")
    assert log.index("preserve a") < log.index("release a")


def test_preserve_runs_only_on_failure() -> None:
    """A passing run has no evidence worth the cost of collecting."""
    log: list[str] = []

    with held(Recorder(log, "a")):
        pass

    assert not [entry for entry in log if entry.startswith("preserve")]


def test_a_broken_preserve_never_replaces_the_operators_failure() -> None:
    """Otherwise the gate reports a diagnostics bug instead of the defect.

    The operator came here to read why the gate failed; a failure inside
    evidence collection is at best a footnote to that.
    """
    log: list[str] = []

    with (
        pytest.raises(GateError, match="the real failure"),
        held(Recorder(log, "a", fail="preserve")),
    ):
        raise GateError("the real failure")

    assert log == ["acquire a", "release a"], "release still happens"


# ---------------------------------------------------------------------------
# Teardown that itself fails
# ---------------------------------------------------------------------------


def test_every_release_is_attempted_before_any_failure_is_raised() -> None:
    """One resource refusing to let go must not strand the others.

    The container that will not stop is exactly the run whose temp directory
    and pidfiles still need clearing.
    """
    log: list[str] = []

    with (
        pytest.raises(GateError, match="would not let go"),
        held(Recorder(log, "a"), Recorder(log, "b", fail="release")),
    ):
        pass

    assert "release a" in log, "a's release must be attempted despite b's failure"


def test_a_teardown_failure_names_the_resource() -> None:
    """`failed to release` on its own sends the reader to the wrong file."""
    log: list[str] = []

    with pytest.raises(GateError) as failure, held(Recorder(log, "gone", fail="release")):
        pass

    assert "recorder" in str(failure.value)


# ---------------------------------------------------------------------------
# Interruption
# ---------------------------------------------------------------------------


def test_an_interrupt_still_releases_and_still_propagates() -> None:
    """Ctrl-C is the path an ad-hoc `finally` most often gets wrong.

    `held` catches `BaseException` rather than `Exception` precisely so an
    interrupted gate tears down -- and re-raises, so the interrupt is never
    reported as a pass.
    """
    log: list[str] = []

    with pytest.raises(KeyboardInterrupt), held(Recorder(log, "a"), Recorder(log, "b")):
        raise KeyboardInterrupt

    assert log[-2:] == ["release b", "release a"]
    assert "preserve b" in log, "an aborted run is worth evidence too"


# ---------------------------------------------------------------------------
# The subclass contract
# ---------------------------------------------------------------------------


def test_a_subclass_must_name_itself() -> None:
    """The name is what a teardown failure message says failed.

    Required as a class keyword rather than defaulted, so forgetting it is a
    `TypeError` at import instead of a teardown error that says `resource`.
    """
    with pytest.raises(TypeError, match="name"):
        # The omission is the subject of the test, so the type checker's
        # agreement that it is wrong is confirmation rather than a problem.
        class Unnamed(Resource):  # ty: ignore[missing-argument]
            def acquire(self) -> None: ...

            def release(self) -> None: ...


def test_a_subclass_must_implement_both_halves() -> None:
    """Acquire without release is the leak this module exists to prevent."""

    class HalfDone(Resource, name="half-done"):
        def acquire(self) -> None: ...

    with pytest.raises(TypeError, match="release"):
        HalfDone()  # ty: ignore[invalid-argument-type]


def test_preserve_is_optional() -> None:
    """Most resources have nothing to save, and forcing an empty override on
    all of them teaches readers to skip the method that matters."""
    log: list[str] = []

    class Plain(Resource, name="plain"):
        def acquire(self) -> None:
            log.append("acquire")

        def release(self) -> None:
            log.append("release")

    with pytest.raises(GateError), held(Plain()):
        raise GateError("boom")

    assert log == ["acquire", "release"]
