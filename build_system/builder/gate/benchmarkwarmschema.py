"""Original paused guest continuation; never cold start as warm success."""
from pydantic import Field, PositiveFloat, PositiveInt, model_validator

from .benchmarkfiniteschema import FiniteEndpointEvidence
from .configschema import Strict


class PauseResumeConfig(Strict):
    cycles: PositiveInt = Field(le=4)
    quick_cycles: PositiveInt = Field(le=4)
    timeout_seconds: PositiveFloat = Field(le=600,allow_inf_nan=False)
    exec_timeout_seconds: PositiveInt = Field(le=60)
    max_response_bytes: PositiveInt = Field(le=1048576)
    max_metadata_bytes: PositiveInt = Field(le=65536)
    max_checkpoint_bytes: PositiveInt = Field(le=17179869184)
    max_terminal_checkpoint_bytes: PositiveInt = Field(le=67108864)
    max_probe_frames: PositiveInt = Field(le=4096)
    max_probe_bytes: PositiveInt = Field(le=16777216)
    close_timeout_seconds: PositiveFloat = Field(le=10,allow_inf_nan=False)
    poll_seconds: PositiveFloat = Field(le=1,allow_inf_nan=False)
    ram_mb: PositiveInt
    cpus: PositiveInt
    name: str = Field(pattern=r'^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$')
    endpoints: dict[str,FiniteEndpointEvidence] = Field(min_length=2,max_length=2)

    @model_validator(mode='after')
    def original_pair(self):
        expected = {'POST /vms/{id}/pause':{'original-checkpoint':'cycles'},
            'POST /vms/{id}/resume':{'original-warm-continuation':'cycles'}}
        if self.quick_cycles > self.cycles or self.exec_timeout_seconds >= self.timeout_seconds:
            raise ValueError('warm cycle/deadline invalid')
        if {key:value.cases for key,value in self.endpoints.items()} != expected:
            raise ValueError('pause/resume require exact original finite pair')
        return self
