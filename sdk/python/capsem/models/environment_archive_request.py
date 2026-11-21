"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictStr

from .environment_archive_action import EnvironmentArchiveAction
from .model_base import Model, UuidString


class EnvironmentArchiveRequest(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    action: EnvironmentArchiveAction
    base_revision_id: UuidString
    reason: StrictStr
    request_id: UuidString
