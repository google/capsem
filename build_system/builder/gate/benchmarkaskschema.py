"""Finite original HTTP approval observations; no synthetic pending rows."""
from pydantic import Field, PositiveFloat, PositiveInt, model_validator

from .benchmarkfiniteschema import FiniteEndpointEvidence
from .configschema import Strict


class AskRouteConfig(Strict):
    cycles: PositiveInt = Field(le=4)
    quick_cycles: PositiveInt = Field(le=4)
    timeout_seconds: PositiveFloat = Field(le=300,allow_inf_nan=False)
    exec_timeout_seconds: PositiveInt = Field(le=60)
    max_response_bytes: PositiveInt = Field(le=1048576)
    max_log_bytes: PositiveInt = Field(le=1048576)
    poll_seconds: PositiveFloat = Field(le=1,allow_inf_nan=False)
    ram_mb: PositiveInt
    cpus: PositiveInt
    endpoints: dict[str,FiniteEndpointEvidence] = Field(min_length=1,max_length=1)

    @model_validator(mode='after')
    def finite_original(self):
        operation = 'POST /vms/{id}/security/asks/{ask_id}/resolve'
        if self.quick_cycles > self.cycles or self.exec_timeout_seconds >= self.timeout_seconds:
            raise ValueError('approval cycle/deadline invalid')
        if set(self.endpoints) != {operation} or self.endpoints[operation].cases != {'approve-original':'cycles','deny-original':'cycles'}:
            raise ValueError('approval requires original approve and deny cases')
        return self
