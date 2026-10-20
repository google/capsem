"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictStr

from .model_base import Model, UuidString


class AuthorizeGoogleRequest(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    reconnect_connection_id: StrictStr | None
    request_id: UuidString
    scope_profile_id: StrictStr
