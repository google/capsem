"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from enum import StrEnum


class TransactionState(StrEnum):
    PENDING = 'pending'
    CONNECTED = 'connected'
    DENIED = 'denied'
    EXPIRED = 'expired'
    CANCELLED = 'cancelled'
    FAILED = 'failed'
