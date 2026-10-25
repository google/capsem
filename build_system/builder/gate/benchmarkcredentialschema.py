"""Finite synthetic broker mutations, never high-RPS credential writes."""
from pydantic import Field, PositiveFloat, PositiveInt, model_validator

from .benchmarkfiniteschema import FiniteEndpointEvidence
from .configschema import Strict


class CredentialRouteConfig(Strict):
    cycles: PositiveInt = Field(le=4)
    quick_cycles: PositiveInt = Field(le=4)
    timeout_seconds: PositiveFloat = Field(le=600,allow_inf_nan=False)
    exec_timeout_seconds: PositiveInt = Field(le=60)
    max_response_bytes: PositiveInt = Field(le=1048576)
    max_log_bytes: PositiveInt = Field(le=1048576)
    max_store_bytes: PositiveInt = Field(le=65536)
    poll_seconds: PositiveFloat = Field(le=1,allow_inf_nan=False)
    ram_mb: PositiveInt
    cpus: PositiveInt
    endpoints: dict[str,FiniteEndpointEvidence] = Field(min_length=2,max_length=2)

    @model_validator(mode='after')
    def finite_credentials(self):
        expected = {'POST /credentials/inject':{'memory','file'},
            'POST /plugins/credential_broker/credentials/reload':{'changed-store'}}
        if self.quick_cycles > self.cycles or self.exec_timeout_seconds >= self.timeout_seconds:
            raise ValueError('credential cycle/deadline invalid')
        if set(self.endpoints) != set(expected) or any(set(self.endpoints[key].cases) != cases or
            set(self.endpoints[key].cases.values()) != {'cycles'} for key,cases in expected.items()):
            raise ValueError('credential cases must bind exact injection storage and changed-store reload')
        return self
