"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from enum import StrEnum


class AgentOperation(StrEnum):
    DISCOVER = 'discover'
    INSPECT = 'inspect'
    PROPOSE = 'propose'
    CANCEL_OWN = 'cancel_own'
    EXECUTE = 'execute'
    LAUNCH = 'launch'
    MANAGE_OWNED_SESSIONS = 'manage_owned_sessions'
