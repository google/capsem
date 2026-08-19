"""Typed policy for evidence-derived gate timing regressions."""

from __future__ import annotations

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
