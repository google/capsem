"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from enum import StrEnum


class McpPermissionAction(StrEnum):
    ALLOW = 'allow'
    ASK = 'ask'
    BLOCK = 'block'
    PREPROCESS = 'preprocess'
    REWRITE = 'rewrite'
    POSTPROCESS = 'postprocess'
