"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from enum import StrEnum


class ManagedSessionState(StrEnum):
    PENDING = 'pending'
    ACTIVE = 'active'
    CLOSING = 'closing'
    CLOSED = 'closed'
    FAILED = 'failed'
