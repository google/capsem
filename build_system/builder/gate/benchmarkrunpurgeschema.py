"""Two finite disposable execution/removal operations, never read-load loops."""
from typing import Literal

from pydantic import Field, PositiveFloat, PositiveInt, model_validator

from .benchmarkfiniteschema import FiniteEndpointEvidence
from .configschema import Strict


class RunPurgeConfig(Strict):
    cycles: PositiveInt = Field(le=4)
    quick_cycles: PositiveInt = Field(le=4)
    timeout_seconds: PositiveFloat = Field(le=600,allow_inf_nan=False)
    exec_timeout_seconds: PositiveInt = Field(le=60)
    max_response_bytes: PositiveInt = Field(le=1048576)
    poll_seconds: PositiveFloat = Field(le=1,allow_inf_nan=False)
    ram_mb: PositiveInt
    cpus: PositiveInt
    retained_sessions: Literal[2]
    marker: str = Field(pattern=r'^[A-Za-z0-9_-]{1,128}$')
    endpoints: dict[str,FiniteEndpointEvidence] = Field(min_length=2,max_length=2)

    @model_validator(mode='after')
    def finite_operations(self):
        expected = {'POST /run':{'vm-fenced-execution-cleanup':'cycles'},
            'POST /purge':{'owned-stopped-disposal':'cycles'}}
        if self.quick_cycles > self.cycles or self.exec_timeout_seconds >= self.timeout_seconds:
            raise ValueError('run/purge cycle/deadline invalid')
        if set(self.endpoints) != set(expected) or any(self.endpoints[key].cases != cases for key,cases in expected.items()):
            raise ValueError('run/purge require exact finite operations and cases')
        return self
