"""Finite prepared image mutations stay separate from concurrent polling."""
from typing import Literal

from pydantic import Field, PositiveFloat, PositiveInt

from .configschema import Strict


class ImageEndpointEvidence(Strict):
    collector: Literal['routes']
    target_scope: Literal['both']
    reason: str = Field(min_length=20)
    cases: dict[str, Literal['cycles', 'cached_samples', 'refresh_samples', 'inventory_samples', 'warm_pulls']] = Field(min_length=1)


class ImageRouteConfig(Strict):
    cycles: PositiveInt = Field(le=10)
    quick_cycles: PositiveInt = Field(le=10)
    cached_samples: PositiveInt
    quick_cached_samples: PositiveInt
    refresh_samples: PositiveInt = Field(le=10)
    quick_refresh_samples: PositiveInt = Field(le=10)
    inventory_samples: PositiveInt
    quick_inventory_samples: PositiveInt
    warm_pulls: PositiveInt = Field(le=10)
    quick_warm_pulls: PositiveInt = Field(le=10)
    timeout_seconds: PositiveFloat = Field(allow_inf_nan=False)
    inspect_timeout_seconds: PositiveFloat = Field(allow_inf_nan=False)
    poll_seconds: PositiveFloat = Field(allow_inf_nan=False)
    max_response_bytes: PositiveInt
    endpoints: dict[str, ImageEndpointEvidence]
