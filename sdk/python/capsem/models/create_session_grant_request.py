"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictStr

from .model_base import DateTimeString, Model, UuidString


class CreateSessionGrantRequest(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    capability_ids: list[StrictStr]
    connection_id: StrictStr
    expected_owner_generation: UuidString
    expires_at: DateTimeString | None
    request_id: UuidString
