"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from enum import StrEnum


class VmLifecycleState(StrEnum):
    RUNNING = 'Running'
    STOPPED = 'Stopped'
    SUSPENDED = 'Suspended'
    DEFUNCT = 'Defunct'
    INCOMPATIBLE = 'Incompatible'
