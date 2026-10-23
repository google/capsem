"""Finite owned lifecycle mutations are not concurrent polling."""
from pydantic import Field, PositiveFloat, PositiveInt, model_validator

from .benchmarkfiniteschema import FiniteEndpointEvidence
from .configschema import Strict


class LifecycleRouteConfig(Strict):
    cycles: PositiveInt = Field(le=10)
    quick_cycles: PositiveInt = Field(le=10)
    timeout_seconds: PositiveFloat = Field(allow_inf_nan=False)
    poll_seconds: PositiveFloat = Field(allow_inf_nan=False)
    max_response_bytes: PositiveInt
    ram_mb: PositiveInt
    cpus: PositiveInt
    exec_timeout_seconds: PositiveInt = Field(le=300)
    marker: str = Field(pattern=r'^[a-zA-Z0-9-]{1,64}$')
    endpoints: dict[str, FiniteEndpointEvidence] = Field(min_length=6, max_length=6)

    @model_validator(mode='after')
    def finite_cases(self):
        if self.quick_cycles > self.cycles:
            raise ValueError('quick lifecycle cycles exceed full cycles')
        if any(len(entry.cases) != 1 or set(entry.cases.values()) != {'cycles'} for entry in self.endpoints.values()):
            raise ValueError('lifecycle requires one finite cycle case per operation')
        return self
