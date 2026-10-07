"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from enum import StrEnum


class ImageCacheState(StrEnum):
    UNKNOWN = 'unknown'
    MISSING = 'missing'
    PARTIAL = 'partial'
    READY = 'ready'
