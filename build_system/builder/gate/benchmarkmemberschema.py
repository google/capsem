"""Bounded genuine private member/cable effects, not generated flow history."""
from pydantic import Field, PositiveFloat, PositiveInt, model_validator

from .benchmarkfiniteschema import FiniteEndpointEvidence
from .configschema import Strict


class NetworkMemberConfig(Strict):
    cycles: PositiveInt = Field(le=4)
    quick_cycles: PositiveInt = Field(le=4)
    timeout_seconds: PositiveFloat = Field(le=600,allow_inf_nan=False)
    exec_timeout_seconds: PositiveInt = Field(le=60)
    max_response_bytes: PositiveInt = Field(le=2097152)
    poll_seconds: PositiveFloat = Field(le=1,allow_inf_nan=False)
    ram_mb: PositiveInt
    cpus: PositiveInt
    name: str = Field(pattern=r'^[a-z][a-z0-9-]{0,61}[a-z0-9]$')
    probe_count: PositiveInt = Field(le=10)
    probe_interval_ms: PositiveInt = Field(le=1000)
    probe_timeout_ms: PositiveInt = Field(le=5000)
    log_limit: PositiveInt = Field(ge=4,le=100)
    endpoints: dict[str,FiniteEndpointEvidence] = Field(min_length=3,max_length=3)

    @model_validator(mode='after')
    def finite_members(self):
        if self.quick_cycles > self.cycles or self.exec_timeout_seconds >= self.timeout_seconds:
            raise ValueError('member cycle/time budget invalid')
        expected = {'PUT /networks/{id}/members/{vm_id}', 'DELETE /networks/{id}/members/{vm_id}', 'GET /networks/{id}/logs'}
        if set(self.endpoints) != expected or any(len(entry.cases) != 1 or set(entry.cases.values()) != {'cycles'} for entry in self.endpoints.values()):
            raise ValueError('member cases must bind exactly attach detach and populated link logs')
        return self
