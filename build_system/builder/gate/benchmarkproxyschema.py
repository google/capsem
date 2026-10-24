"""Bounded original standalone leases, distinct from concurrent read polling."""
from typing import Literal

from pydantic import Field, PositiveFloat, PositiveInt, model_validator

from .benchmarkfiniteschema import FiniteEndpointEvidence
from .configschema import Strict


class ProxyRouteConfig(Strict):
    cycles: PositiveInt = Field(le=10)
    quick_cycles: PositiveInt = Field(le=10)
    timeout_seconds: PositiveFloat = Field(le=120, allow_inf_nan=False)
    max_response_bytes: PositiveInt = Field(le=1048576)
    max_log_bytes: PositiveInt = Field(le=1048576)
    lease_ttl_seconds: Literal[15]
    renew_after_seconds: PositiveFloat = Field(allow_inf_nan=False)
    expiry_margin_seconds: PositiveFloat = Field(allow_inf_nan=False)
    poll_seconds: PositiveFloat = Field(le=1, allow_inf_nan=False)
    provider: Literal['openai']
    worker_binary: Literal['capsem-proxy']
    model: str = Field(min_length=1, max_length=100)
    expected_content: str = Field(min_length=1, max_length=4096)
    endpoints: dict[str, FiniteEndpointEvidence] = Field(min_length=3, max_length=3)

    @model_validator(mode='after')
    def bounded_lease(self):
        if self.quick_cycles > self.cycles:
            raise ValueError('quick proxy cycles exceed full cycles')
        if not self.expiry_margin_seconds < self.renew_after_seconds < self.lease_ttl_seconds:
            raise ValueError('renewal must leave a real post-original-expiry window')
        if self.timeout_seconds <= self.lease_ttl_seconds + self.renew_after_seconds + self.expiry_margin_seconds:
            raise ValueError('proxy timeout must cover TTL plus renewal and cleanup')
        if set(self.endpoints) != {'POST /proxies', 'POST /proxies/{id}/heartbeat', 'POST /proxies/{id}/stop'}:
            raise ValueError('proxy cases must bind three original lifecycle operations')
        if any(len(e.cases) != 1 or set(e.cases.values()) != {'cycles'} for e in self.endpoints.values()):
            raise ValueError('proxy cases require one finite cycle')
        return self
