"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from enum import StrEnum


class ConnectionState(StrEnum):
    CONNECTED = 'connected'
    REAUTH_REQUIRED = 'reauth_required'
    DISCONNECTING = 'disconnecting'
    DISCONNECTED = 'disconnected'
    DEGRADED = 'degraded'
