"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from enum import StrEnum


class ContainerState(StrEnum):
    PULLING = 'pulling'
    STAGING = 'staging'
    STAGED = 'staged'
    STARTING = 'starting'
    PREPARING = 'preparing'
    RUNNING = 'running'
    EXITED = 'exited'
    FAILED = 'failed'
