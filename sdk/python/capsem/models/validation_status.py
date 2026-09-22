"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from enum import StrEnum


class ValidationStatus(StrEnum):
    VALID = 'valid'
    INVALID = 'invalid'
    MISSING = 'missing'
    FETCH_ERROR = 'fetch_error'
