"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from enum import StrEnum


class RequestKind(StrEnum):
    ENVIRONMENT_CHANGE = 'environment_change'
    ENVIRONMENT_IMAGE_UPDATE = 'environment_image_update'
    ENVIRONMENT_PROMOTION = 'environment_promotion'
    ENVIRONMENT_APPLY = 'environment_apply'
