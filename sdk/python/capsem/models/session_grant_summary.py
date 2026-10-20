"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import ConfigDict, Field, StrictBool, StrictInt, StrictStr

from .grant_state import GrantState
from .model_base import DateTimeString, Model


class SessionGrantSummary(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    capability_ids: list[StrictStr]
    connection_id: StrictStr
    created_at: DateTimeString
    expires_at: DateTimeString
    id: StrictStr
    owner_generation: StrictStr
    revision: Annotated[Annotated[StrictInt, Field(ge=0)], Field(le=9007199254740991)]
    revoked_at: DateTimeString | None
    runtime_bound: StrictBool
    session_id: StrictStr
    state: GrantState
