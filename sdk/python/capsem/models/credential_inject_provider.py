"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from enum import StrEnum


class CredentialInjectProvider(StrEnum):
    ANTHROPIC = 'anthropic'
    GOOGLE = 'google'
    OPENAI = 'openai'
    GITHUB = 'github'
    MCP = 'mcp'
