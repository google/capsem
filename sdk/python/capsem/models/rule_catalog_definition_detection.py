"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Literal

from .environment_detection_rule import EnvironmentDetectionRule
from .model_base import Model


class RuleCatalogDefinitionDetection(Model):
    kind: Literal["detection"]
    rule: EnvironmentDetectionRule
