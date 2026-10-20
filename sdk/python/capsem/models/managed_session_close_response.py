"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictBool, StrictStr

from .managed_session_closed_state import ManagedSessionClosedState
from .model_base import Model


class ManagedSessionCloseResponse(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    evidence_retained: StrictBool
    generation: StrictStr
    request_id: StrictStr
    session_id: StrictStr | None = None
    state: ManagedSessionClosedState
