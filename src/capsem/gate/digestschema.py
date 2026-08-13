"""What the run ledger keeps, and what the digest is allowed to conclude.

Split from `harnessschema` for the reason every split here happens: that module
is at its own boundary. These two settings groups are also the ones most likely
to be tuned, and tuning a threshold should not mean opening the file that
describes every other harness.

No advice text lives in code. A rule says which measurement it reads and what
factor makes it worth mentioning; the sentence it produces is assembled from
the measurement, so a threshold and its wording cannot drift apart.
"""

from __future__ import annotations

from pydantic import PositiveFloat, PositiveInt

from .configschema import Strict


class LedgerConfig(Strict):
    """The distilled record that outlives the run directories.

    `keep_runs` is twenty, and a day of gating spends that before lunch. Every
    question worth asking about the gate -- is this step getting slower, does
    that one keep failing, did the reordering help -- is a question across
    runs, and all of them were answerable only over whatever rotation had not
    yet reached. A row is a couple of kilobytes, so months of history costs
    less than one run's event log.
    """

    path: str
    row_schema: str
    keep_rows: PositiveInt


class DigestConfig(Strict):
    """How the overview is written, and when it is willing to give advice."""

    path: str

    compare_runs: PositiveInt
    """How many comparable runs form the baseline a trend is measured against.

    Comparable in the ledger's sense: same command, argv, host class and plan
    shape. Pooling anything else compares a hundred-step candidate with a
    six-step install and calls the difference a regression.
    """

    recent_runs: PositiveInt
    """How many runs the status table shows, regardless of comparability."""

    regression_factor: PositiveFloat
    """Slower than the comparable median by this much before it is mentioned.

    Deliberately looser than the release ratchet's `maximum_factor`. That one
    refuses a release; this one writes a sentence, and a sentence that fires on
    ordinary variance is a sentence people stop reading.
    """

    improvement_factor: PositiveFloat
    """Faster than the comparable median by this much before it is mentioned.

    Improvements are reported for the same reason regressions are: a change
    made to speed something up is a claim, and the digest is where it is either
    confirmed or quietly not.
    """

    thrash_runs: PositiveInt
    """Failures across the compared window before a step is called unreliable.

    Two is not noise and is not yet a pattern; the point of naming it is that a
    step failing intermittently costs more than one failing outright, because
    nobody stops the line for it.
    """

    residency_fraction: PositiveFloat
    """Share of comparable runs a step must own the critical path to be a
    hotspot. Being slow is not the same as being in the way."""

    wait_fraction: PositiveFloat
    """Share of a step's elapsed time spent waiting on resources before the
    advice changes from "make it faster" to "it is queueing".

    The distinction matters more than the number: a critical path made of
    waiting does not get shorter when its steps do.
    """

    lane_share: PositiveFloat
    """Share of a lane's critical path a single `FAST` step may own."""

    hotspots: PositiveInt
    """How many hotspots the digest names. A ranked list nobody finishes is a
    list that ranked nothing."""
