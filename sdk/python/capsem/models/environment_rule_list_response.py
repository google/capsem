"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict

from .environment_effective_access_rule import EnvironmentEffectiveAccessRule
from .environment_effective_detection_rule import EnvironmentEffectiveDetectionRule
from .environment_revision_reference import EnvironmentRevisionReference
from .model_base import Model


class EnvironmentRuleListResponse(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    access: list[EnvironmentEffectiveAccessRule]
    detection: list[EnvironmentEffectiveDetectionRule]
    revision: EnvironmentRevisionReference
