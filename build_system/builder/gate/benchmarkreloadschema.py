"""Finite live corporate reload effects on one original owner."""
from pydantic import Field, PositiveFloat, PositiveInt, model_validator

from .benchmarkfiniteschema import FiniteEndpointEvidence
from .configschema import Strict


class ReloadRouteConfig(Strict):
    cycles: PositiveInt = Field(le=4)
    quick_cycles: PositiveInt = Field(le=4)
    timeout_seconds: PositiveFloat = Field(le=300,allow_inf_nan=False)
    exec_timeout_seconds: PositiveInt = Field(le=60)
    max_response_bytes: PositiveInt = Field(le=1048576)
    max_log_bytes: PositiveInt = Field(le=1048576)
    max_policy_bytes: PositiveInt = Field(le=4194304)
    poll_seconds: PositiveFloat = Field(le=1,allow_inf_nan=False)
    ram_mb: PositiveInt
    cpus: PositiveInt
    name: str = Field(pattern=r'^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$')
    endpoints: dict[str,FiniteEndpointEvidence] = Field(min_length=1,max_length=1)

    @model_validator(mode='after')
    def finite_original(self):
        operation = 'POST /corp/reload'
        if self.quick_cycles > self.cycles or self.exec_timeout_seconds >= self.timeout_seconds:
            raise ValueError('reload cycle/deadline invalid')
        if set(self.endpoints) != {operation} or self.endpoints[operation].cases != {'block-original':'cycles','restore-original':'cycles'}:
            raise ValueError('reload requires original block and restore cases')
        return self
