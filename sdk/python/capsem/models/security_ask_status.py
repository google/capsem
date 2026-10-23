"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from enum import StrEnum


class SecurityAskStatus(StrEnum):
    PENDING = 'pending'
    APPROVED = 'approved'
    DENIED = 'denied'
    EXPIRED = 'expired'
    CANCELLED = 'cancelled'
