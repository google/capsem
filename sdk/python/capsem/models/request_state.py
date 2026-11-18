"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from enum import StrEnum


class RequestState(StrEnum):
    PENDING = 'pending'
    APPROVED = 'approved'
    DENIED = 'denied'
    CANCELLED = 'cancelled'
    EXPIRED = 'expired'
    STALE = 'stale'
