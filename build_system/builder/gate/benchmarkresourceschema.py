"""Budgets for sampled resources of one exact Criterion runtime child."""

from pydantic import Field, PositiveFloat, PositiveInt

from .configschema import Strict


class CriterionResourceConfig(Strict):
    interval_seconds: PositiveFloat = Field(strict=True, allow_inf_nan=False)
    stop_timeout_seconds: PositiveFloat = Field(strict=True, allow_inf_nan=False)
    max_processes: PositiveInt = Field(strict=True)
    max_identities: PositiveInt = Field(strict=True)
    max_observations: PositiveInt = Field(strict=True)
