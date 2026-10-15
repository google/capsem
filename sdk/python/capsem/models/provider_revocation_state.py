"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from enum import StrEnum


class ProviderRevocationState(StrEnum):
    NOT_STARTED = 'not_started'
    PENDING = 'pending'
    SUCCEEDED = 'succeeded'
    FAILED = 'failed'
