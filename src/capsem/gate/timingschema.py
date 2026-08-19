"""Typed policy for evidence-derived gate timing regressions."""

from __future__ import annotations

import os

from pydantic import PositiveFloat, PositiveInt, field_validator

from .configschema import Strict


class TimingRegressionConfig(Strict):
    """Reject slowdowns relative to evidence, never an authored duration."""

    maximum_factor: PositiveFloat
    slowest_steps: PositiveInt

    @field_validator("maximum_factor")
    @classmethod
    def _must_allow_some_variance(cls, factor: float) -> float:
        if factor <= 1.0:
            raise ValueError("maximum_factor must be greater than one")
        return factor


class FastLaneBudget(Strict):
    """An absolute ceiling on the gate a developer waits for.

    Deliberately not `TimingRegressionConfig`, which rejects slowdowns relative
    to evidence and says so: a run twice as slow as the last one is a
    regression, and a run that was always slow is not. Both are true, and only
    one of them tells you whether `fast-test` deserves its name.

    Twenty-one minutes is what it measured when this was written, of which one
    un-parallelised step was ten. A budget is the thing that makes that a
    failure rather than a fact somebody mentions.
    """

    seconds: PositiveFloat
    commands: tuple[str, ...]

    #: The variable whose presence means "not the lane anybody waits for".
    #: A hosted runner starts cold and pays for infrastructure a developer's
    #: machine has cached, so enforcing there fails releases for a Docker
    #: build rather than for a slow test.
    unenforced_when_set: str

    def for_command(self, name: str) -> float | None:
        """What this command may cost, or nothing if its name promises nothing.

        Only the lane `fast-test` dispatches is bounded. A release is allowed
        to take an hour, and failing one on a stopwatch trades a real proof for
        a quick one.
        """
        if name not in self.commands or os.environ.get(self.unenforced_when_set):
            return None
        return self.seconds

    @field_validator("commands")
    @classmethod
    def _names_the_lane_it_bounds(cls, commands: tuple[str, ...]) -> tuple[str, ...]:
        if not commands:
            raise ValueError("a fast-lane budget must name the commands it bounds")
        return commands
