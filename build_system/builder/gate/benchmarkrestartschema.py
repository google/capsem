"""Finite native supervisor case, outside ordinary route confinement."""
from typing import Literal

from pydantic import Field, PositiveFloat, PositiveInt, model_validator

from .configschema import Strict


class RestartEndpointEvidence(Strict):
    collector: Literal['restart']
    target_scope: Literal['both']
    reason: str = Field(min_length=20)
    cases: dict[Literal['managed-original'],Literal['cycles']] = Field(min_length=1,max_length=1)


class RestartRouteConfig(Strict):
    dimension: Literal['restart']
    cycles: PositiveInt = Field(le=4)
    timeout_seconds: PositiveFloat = Field(le=120,allow_inf_nan=False)
    poll_seconds: PositiveFloat = Field(le=1,allow_inf_nan=False)
    max_attempts: PositiveInt = Field(le=1000)
    max_response_bytes: PositiveInt = Field(le=1048576)
    max_roots: PositiveInt = Field(le=128)
    workspace_files: PositiveInt = Field(le=128)
    max_workspace_bytes: PositiveInt = Field(le=1048576)
    max_registry_bytes: PositiveInt = Field(le=1048576)
    endpoints: dict[str,RestartEndpointEvidence] = Field(min_length=1,max_length=1)

    @model_validator(mode='after')
    def original_only(self):
        if set(self.endpoints) != {'POST /restart'}:
            raise ValueError('restart requires exact original operation')
        return self
