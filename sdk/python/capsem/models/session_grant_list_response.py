"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictStr

from .model_base import Model
from .session_grant_summary import SessionGrantSummary


class SessionGrantListResponse(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    denied_capability_ids: list[StrictStr]
    grants: list[SessionGrantSummary]
    owner_generation: StrictStr
    session_id: StrictStr
