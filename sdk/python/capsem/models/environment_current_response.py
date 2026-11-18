"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictInt, StrictStr

from .environment_document import EnvironmentDocument
from .model_base import Model


class EnvironmentCurrentResponse(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    actor: StrictStr
    document: EnvironmentDocument
    reason: StrictStr
    request_id: StrictStr | None = None
    timestamp_unix_ms: StrictInt
