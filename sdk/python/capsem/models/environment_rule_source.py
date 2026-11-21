"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from enum import StrEnum


class EnvironmentRuleSource(StrEnum):
    BUILTIN = 'builtin'
    CORPORATE = 'corporate'
    ENVIRONMENT = 'environment'
    CATALOG = 'catalog'
    ENVIRONMENT_MCP = 'environment_mcp'
