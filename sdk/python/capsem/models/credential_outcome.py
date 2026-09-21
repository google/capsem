"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from enum import StrEnum


class CredentialOutcome(StrEnum):
    CAPTURED = 'captured'
    BROKERED = 'brokered'
    INJECTED = 'injected'
    ERROR = 'error'
