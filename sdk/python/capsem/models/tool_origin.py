"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from enum import StrEnum


class ToolOrigin(StrEnum):
    MODEL = 'model'
    NATIVE = 'native'
    MCP = 'mcp'
    BUILTIN = 'builtin'
    LOCAL = 'local'
    MCP_PROXY = 'mcp_proxy'
