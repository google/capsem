"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictStr

from .connection_summary import ConnectionSummary
from .model_base import Model
from .scope_profile_summary import ScopeProfileSummary


class ConnectionListResponse(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    connections: list[ConnectionSummary]
    next_cursor: StrictStr | None
    scope_profiles: list[ScopeProfileSummary]
