"""Finite host-only settings/policy edits have no network or VM workload."""
from typing import Literal

from pydantic import Field, PositiveFloat, PositiveInt, model_validator

from .benchmarkfiniteschema import FiniteEndpointEvidence
from .configschema import Strict


class PolicyRouteConfig(Strict):
    cycles: PositiveInt = Field(le=10)
    quick_cycles: PositiveInt = Field(le=10)
    timeout_seconds: PositiveFloat = Field(allow_inf_nan=False)
    max_response_bytes: PositiveInt
    file_max_bytes: PositiveInt
    setting_id: Literal['appearance.font_size']
    setting_baseline: int = Field(ge=8, le=32)
    setting_value: int = Field(ge=8, le=32)
    corp_value: int = Field(ge=8, le=32)
    plugin_id: Literal['dummy_pre_eicar']
    plugin_baseline: Literal['block']
    plugin_value: Literal['rewrite']
    default_baseline: Literal['block']
    default_value: Literal['ask']
    server_id: Literal['local']
    tool_id: Literal['fetch_http']
    tool_baseline: Literal['ask']
    tool_value: Literal['block']
    endpoints: dict[str, FiniteEndpointEvidence] = Field(min_length=1, max_length=6)

    @model_validator(mode='after')
    def edits_change_state(self):
        if self.setting_baseline in {self.setting_value, self.corp_value}:
            raise ValueError('policy measurement must change the effective setting')
        if any(len(entry.cases) != 1 or set(entry.cases.values()) != {'cycles'} for entry in self.endpoints.values()):
            raise ValueError('policy mutations require one finite cycle case per operation')
        return self
