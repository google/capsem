"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from enum import StrEnum


class PluginConfiguration_SecretExfiltrationOptionsMode(StrEnum):
    DISABLE = 'disable'
    ALLOW = 'allow'
    ASK = 'ask'
    BLOCK = 'block'
    REWRITE = 'rewrite'
