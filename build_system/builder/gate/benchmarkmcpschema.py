"""Finite live builtin MCP operations; mutations are never read-load loops."""
from pydantic import Field, PositiveFloat, PositiveInt, model_validator

from .benchmarkfiniteschema import FiniteEndpointEvidence
from .configschema import Strict


class BuiltinMetadata(Strict):
    title: str = Field(min_length=1,max_length=100)
    description: str = Field(min_length=1,max_length=1000)


class McpRouteConfig(Strict):
    cycles: PositiveInt = Field(le=4)
    quick_cycles: PositiveInt = Field(le=4)
    timeout_seconds: PositiveFloat = Field(le=600,allow_inf_nan=False)
    exec_timeout_seconds: PositiveInt = Field(le=60)
    max_response_bytes: PositiveInt = Field(le=1048576)
    max_log_bytes: PositiveInt = Field(le=1048576)
    poll_seconds: PositiveFloat = Field(le=1,allow_inf_nan=False)
    ram_mb: PositiveInt
    cpus: PositiveInt
    path: str = Field(pattern=r'^/html/about$')
    expected_content_type: str = Field(min_length=1,max_length=100)
    expected_body: str = Field(min_length=1,max_length=4096)
    tools: dict[str,BuiltinMetadata] = Field(min_length=4,max_length=4)
    endpoints: dict[str,FiniteEndpointEvidence] = Field(min_length=2,max_length=2)

    @model_validator(mode='after')
    def finite_mcp(self):
        expected = {'POST /mcp/servers/{server_id}/refresh':{'discovered-live'},
            'POST /mcp/servers/{server_id}/tools/{tool_id}/call':{'headers','raw-body'}}
        if self.quick_cycles > self.cycles or self.exec_timeout_seconds >= self.timeout_seconds:
            raise ValueError('MCP cycle/deadline invalid')
        if set(self.tools) != {'echo','fetch_http','grep_http','http_headers'}:
            raise ValueError('MCP corpus must bind exactly the four shipped local builtin tools')
        if set(self.endpoints) != set(expected) or any(set(self.endpoints[key].cases) != cases or
            set(self.endpoints[key].cases.values()) != {'cycles'} for key,cases in expected.items()):
            raise ValueError('MCP cases must bind exact refresh and builtin call operations')
        return self
