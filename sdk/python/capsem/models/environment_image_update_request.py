"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictStr

from .environment_image_update_action import EnvironmentImageUpdateAction
from .model_base import Model, UuidString


class EnvironmentImageUpdateRequest(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    action: EnvironmentImageUpdateAction
    base_revision_id: UuidString
    expected_digest: StrictStr
    reason: StrictStr
    request_id: UuidString
