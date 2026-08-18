"""The wall-clock ceiling on a lane whose name promises speed.

Its own module because it is a rule, not a scheduling detail, and because
`planrunner` is at the line boundary this package holds itself to.

Deliberately not `timing_regression`, which compares a run against its own
history: a run twice as slow as the last one is a regression, and a run that
was always slow is not. Both questions are worth asking and only one of them
tells you whether `fast-test` deserves the name.
"""

from __future__ import annotations

import time

from .errors import GateError


def refuse_if_past(began: float, budget: float | None) -> None:
    """Stop a lane that has outrun the budget its name promises.

    Between waves rather than mid-step: a step that is already running owns a
    machine claim and a subprocess, and killing it there would leave both to be
    reasoned about. The next boundary is soon enough to save the wait, and it
    is a place where nothing is half-done.
    """
    if budget is None:
        return
    spent = time.monotonic() - began
    if spent > budget:
        raise GateError(
            f"this lane passed its deadline: {spent:.0f}s spent against a "
            f"{budget:.0f}s budget. It is named for being fast, so a slower "
            "run is a failure rather than a fact -- profile the run log's "
            "slowest actions, or change the budget deliberately."
        )
