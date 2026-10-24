"""Finite genuine workload controls, not concurrent mutation polling."""
from pydantic import Field, PositiveFloat, PositiveInt, model_validator

from .benchmarkfiniteschema import FiniteEndpointEvidence
from .configschema import Strict


class WorkloadControlConfig(Strict):
    cycles: PositiveInt = Field(le=10)
    quick_cycles: PositiveInt = Field(le=10)
    timeout_seconds: PositiveFloat = Field(allow_inf_nan=False,le=600)
    exec_timeout_seconds: PositiveInt = Field(le=60)
    max_response_bytes: PositiveInt = Field(le=4194304)
    poll_seconds: PositiveFloat = Field(allow_inf_nan=False,le=1)
    ram_mb: PositiveInt
    cpus: PositiveInt
    file_sizes: tuple[PositiveInt,...] = Field(min_length=1,max_length=2)
    timeline_limit: PositiveInt = Field(le=200)
    browser_url: str = Field(pattern=r'^https://example\.invalid/[a-z-]+$')
    endpoints: dict[str,FiniteEndpointEvidence] = Field(min_length=2,max_length=2)

    @model_validator(mode='after')
    def finite_cases(self):
        if self.quick_cycles > self.cycles or self.exec_timeout_seconds >= self.timeout_seconds:
            raise ValueError('control sample/time budget invalid')
        if len(set(self.file_sizes)) != len(self.file_sizes) or any(size > 1048576 for size in self.file_sizes):
            raise ValueError('control file corpus must be unique and bounded to1MiB')
        expected = {'POST /vms/{id}/browser/acknowledge':{'original-request-cleared'},
                    'POST /vms/{id}/files/content':{f'bytes-{size}' for size in self.file_sizes}}
        if set(self.endpoints) != set(expected):
            raise ValueError('controls must bind exactly browser acknowledgement and file upload')
        for op,cases in expected.items():
            if set(self.endpoints[op].cases) != cases or set(self.endpoints[op].cases.values()) != {'cycles'}:
                raise ValueError('control cases must match exact finite corpus')
        return self
