"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from enum import StrEnum


class HostLogSource(StrEnum):
    SERVICE = 'service'
    MCP = 'mcp'
    GATEWAY = 'gateway'
    TRAY = 'tray'
    APP = 'app'
