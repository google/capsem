"""Finite prepared image mutations stay separate from concurrent polling."""
from pydantic import Field, PositiveFloat, PositiveInt

from .benchmarkfiniteschema import FiniteEndpointEvidence
from .configschema import Strict


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
    endpoints: dict[str, FiniteEndpointEvidence]
