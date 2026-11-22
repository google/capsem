"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from enum import StrEnum


class CatalogAvailability(StrEnum):
    UNVERIFIED = 'unverified'
    AVAILABLE = 'available'
    STALE = 'stale'
    UNAVAILABLE = 'unavailable'
