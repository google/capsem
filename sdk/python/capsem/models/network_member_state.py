"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from enum import StrEnum


class NetworkMemberState(StrEnum):
    DECLARED = 'declared'
    ATTACHING = 'attaching'
    READY = 'ready'
    FAILED = 'failed'
