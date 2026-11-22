"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Literal

from .environment_access_rule import EnvironmentAccessRule
from .model_base import Model


class RuleCatalogDefinitionAccess(Model):
    kind: Literal["access"]
    rule: EnvironmentAccessRule
