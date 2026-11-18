"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from enum import StrEnum


class RequestApplicationState(StrEnum):
    PENDING = 'pending'
    APPLYING = 'applying'
    APPLIED = 'applied'
    FAILED = 'failed'
