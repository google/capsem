"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from enum import StrEnum


class ToolDecision(StrEnum):
    ALLOWED = 'allowed'
    DENIED = 'denied'
    WARNED = 'warned'
    ERROR = 'error'
