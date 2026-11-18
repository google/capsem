"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictBool

from .environment_capability_decision import EnvironmentCapabilityDecision
from .model_base import Model


class EnvironmentMcpPermissions(Model):
    nonnullable_optional = frozenset(['default', 'enabled', 'tools'])
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    default: EnvironmentCapabilityDecision | None = None
    enabled: StrictBool | None = None
    tools: dict[str, EnvironmentCapabilityDecision] | None = None
