"""One finite real check; native apply remains a separate installed owner."""
from pydantic import Field, PositiveFloat, PositiveInt, model_validator

from .benchmarkfiniteschema import FiniteEndpointEvidence
from .configschema import Strict


class UpdateCheckConfig(Strict):
    cycles: PositiveInt = Field(le=4)
    quick_cycles: PositiveInt = Field(le=4)
    timeout_seconds: PositiveFloat = Field(le=120,allow_inf_nan=False)
    max_response_bytes: PositiveInt = Field(le=1048576)
    max_manifest_bytes: PositiveInt = Field(le=4194304)
    max_cli_output_bytes: PositiveInt = Field(le=65536)
    binary_version: str = Field(pattern=r'^\d+\.\d+\.\d+$')
    runtime_revision: str = Field(pattern=r'^\d{4}\.\d{4}\.\d+$')
    manifest_path: str = Field(pattern=r'^/fixtures/[a-z-]+/manifest\.json$')
    endpoints: dict[str,FiniteEndpointEvidence] = Field(min_length=1,max_length=1)

    @model_validator(mode='after')
    def genuine_original(self):
        if self.quick_cycles > self.cycles or self.max_cli_output_bytes >= self.max_response_bytes:
            raise ValueError('update check cycles/output bounds invalid')
        if {key:value.cases for key,value in self.endpoints.items()} != {
            'POST /update/check':{'real-cli-manifest-fetch':'cycles'}}:
            raise ValueError('update check needs the original real CLI finite case')
        return self
