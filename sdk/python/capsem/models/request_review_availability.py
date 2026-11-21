"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from enum import StrEnum


class RequestReviewAvailability(StrEnum):
    COMPLETE = 'complete'
    PURGED = 'purged'
    UNAVAILABLE = 'unavailable'
