"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictStr

from .environment_apply_target import EnvironmentApplyTarget
from .model_base import Model, UuidString


class EnvironmentApplyRequest(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    base_revision_id: UuidString
    reason: StrictStr
    request_id: UuidString
    revision_id: UuidString
    targets: list[EnvironmentApplyTarget]
