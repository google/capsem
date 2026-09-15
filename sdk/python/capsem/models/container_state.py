"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from enum import StrEnum


class ContainerState(StrEnum):
    PULLING = 'pulling'
    STAGING = 'staging'
    STARTING = 'starting'
    RUNNING = 'running'
    EXITED = 'exited'
    FAILED = 'failed'
