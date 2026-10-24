"""One original ephemeral-to-persistent save, with bounded cold verification."""
from pydantic import Field, PositiveFloat, PositiveInt, model_validator

from .benchmarkfiniteschema import FiniteEndpointEvidence
from .configschema import Strict


class SaveRouteConfig(Strict):
    cycles: PositiveInt = Field(le=4)
    quick_cycles: PositiveInt = Field(le=4)
    timeout_seconds: PositiveFloat = Field(le=600,allow_inf_nan=False)
    exec_timeout_seconds: PositiveInt = Field(le=60)
    max_response_bytes: PositiveInt = Field(le=1048576)
    poll_seconds: PositiveFloat = Field(le=1,allow_inf_nan=False)
    ram_mb: PositiveInt
    cpus: PositiveInt
    name: str = Field(pattern=r'^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$')
    marker: str = Field(pattern=r'^[A-Za-z0-9_-]{1,128}$')
    endpoints: dict[str,FiniteEndpointEvidence] = Field(min_length=1,max_length=1)

    @model_validator(mode='after')
    def finite_save(self):
        if self.quick_cycles > self.cycles or self.exec_timeout_seconds >= self.timeout_seconds:
            raise ValueError('save cycle/deadline invalid')
        if set(self.endpoints) != {'POST /vms/{id}/save'} or self.endpoints['POST /vms/{id}/save'].cases != {'original-live-claim':'cycles'}:
            raise ValueError('save requires exact original finite mutation')
        return self
