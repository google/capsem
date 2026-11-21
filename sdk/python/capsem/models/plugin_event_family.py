"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from enum import StrEnum


class PluginEventFamily(StrEnum):
    HTTP = 'http'
    MODEL = 'model'
    MCP = 'mcp'
    FILE = 'file'
    CREDENTIAL = 'credential'
    NETWORK = 'network'
    DNS = 'dns'
    PROCESS = 'process'
    SECURITY = 'security'
