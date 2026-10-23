"""Finite empty-network effects and bounded labelled synthetic diagnostic input."""
from datetime import datetime
from itertools import pairwise

from pydantic import Field, PositiveFloat, PositiveInt, model_validator

from .benchmarkfiniteschema import FiniteEndpointEvidence
from .configschema import Strict


class NetworkRouteConfig(Strict):
    cycles: PositiveInt = Field(le=10)
    quick_cycles: PositiveInt = Field(le=10)
    timeout_seconds: PositiveFloat = Field(allow_inf_nan=False)
    max_response_bytes: PositiveInt
    name: str = Field(pattern=r'^[a-zA-Z0-9-]{1,64}$')
    endpoints: dict[str, FiniteEndpointEvidence] = Field(min_length=2, max_length=2)

    @model_validator(mode='after')
    def finite_cases(self):
        if self.quick_cycles > self.cycles:
            raise ValueError('quick network cycles exceed full cycles')
        if set(self.endpoints) != {'POST /networks', 'DELETE /networks/{id}'}:
            raise ValueError('network cases must bind exactly create and retire')
        if any(len(entry.cases) != 1 or set(entry.cases.values()) != {'cycles'} for entry in self.endpoints.values()):
            raise ValueError('network operation requires one finite cycle case')
        return self


class DiagnosticCorpusConfig(Strict):
    old_timestamp: str
    since: str
    first_timestamp: str
    later_since: str
    second_timestamp: str
    warning_timestamp: str
    limit: PositiveInt = Field(ge=3, le=200)
    narrow_limit: PositiveInt = Field(le=200)
    padding_lines: PositiveInt = Field(le=10000)
    padding_bytes: PositiveInt = Field(le=4096)
    max_file_bytes: PositiveInt = Field(le=1048576)

    @model_validator(mode='after')
    def ordered_windows(self):
        values = [self.old_timestamp, self.since, self.first_timestamp, self.later_since, self.second_timestamp, self.warning_timestamp]
        if any(not value.endswith('Z') for value in values):
            raise ValueError('diagnostic timestamps must use UTC Z')
        times = [datetime.fromisoformat(value) for value in values]
        if any(left >= right for left, right in pairwise(times)):
            raise ValueError('diagnostic timestamps must increase across since windows')
        if self.narrow_limit != 1:
            raise ValueError('narrow diagnostic case must truncate to one event')
        return self
