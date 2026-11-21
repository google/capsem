"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictBool, StrictInt, StrictStr

from .environment_revision_reference import EnvironmentRevisionReference
from .model_base import Model


class EnvironmentRevisionSummary(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    actor: StrictStr
    environment: EnvironmentRevisionReference
    fork_parent: EnvironmentRevisionReference | None = None
    previous_revision_id: StrictStr | None = None
    reason: StrictStr
    replayable: StrictBool
    request_id: StrictStr | None = None
    timestamp_unix_ms: StrictInt
