"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from enum import StrEnum


class PluginStage(StrEnum):
    PREPROCESS = 'preprocess'
    POSTPROCESS = 'postprocess'
    LOGGING = 'logging'
