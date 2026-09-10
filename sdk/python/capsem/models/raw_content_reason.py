"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from enum import StrEnum


class RawContentReason(StrEnum):
    TRUNCATED = 'truncated'
    INVALID_JSON = 'invalid_json'
    UNPARSED = 'unparsed'
