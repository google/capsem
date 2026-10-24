"""Finite owned preview controls, with gateway-only capability minting."""
from typing import Literal

from pydantic import Field, PositiveFloat, PositiveInt, model_validator

from .configschema import Strict


class PreviewEndpoint(Strict):
    collector: Literal['routes']
    target_scope: Literal['both','gateway']
    reason: str = Field(min_length=20)
    cases: dict[str,Literal['cycles']] = Field(min_length=1,max_length=1)


class PreviewRouteConfig(Strict):
    cycles: PositiveInt = Field(le=4)
    quick_cycles: PositiveInt = Field(le=4)
    timeout_seconds: PositiveFloat = Field(le=600,allow_inf_nan=False)
    exec_timeout_seconds: PositiveInt = Field(le=60)
    max_response_bytes: PositiveInt = Field(le=1048576)
    max_token_bytes: PositiveInt = Field(le=4096)
    bootstrap_lifetime_seconds: Literal[30]
    poll_seconds: PositiveFloat = Field(le=1,allow_inf_nan=False)
    ram_mb: PositiveInt
    cpus: PositiveInt
    endpoints: dict[str,PreviewEndpoint] = Field(min_length=5,max_length=5)

    @model_validator(mode='after')
    def finite_controls(self):
        expected = {
            'POST /vms/{id}/exposures':('both','owned-declaration'),
            'DELETE /vms/{id}/exposures/{exposure_id}':('both','original-flow-deleted'),
            'POST /vms/{id}/exposures/{exposure_id}/preview-session':('gateway','single-use-capability'),
            'DELETE /vms/{id}/exposures/{exposure_id}/preview-session':('both','original-flow-revoked'),
            'POST /vms/{id}/surface/session':('gateway','declared-port-wiring'),
        }
        if self.quick_cycles > self.cycles or self.exec_timeout_seconds >= self.timeout_seconds:
            raise ValueError('preview cycle/deadline invalid')
        if set(self.endpoints) != set(expected) or any(self.endpoints[op].target_scope != scope or
                self.endpoints[op].cases != {case:'cycles'} for op,(scope,case) in expected.items()):
            raise ValueError('preview cases require exact operations and gateway-only minting')
        return self
