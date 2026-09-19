"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from enum import StrEnum


class VmAction(StrEnum):
    PAUSE = 'pause'
    STOP = 'stop'
    START = 'start'
    RESUME = 'resume'
    FORK = 'fork'
    DELETE = 'delete'
