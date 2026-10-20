"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from enum import StrEnum


class GrantState(StrEnum):
    ACTIVE = 'active'
    REVOKED = 'revoked'
    EXPIRED = 'expired'
