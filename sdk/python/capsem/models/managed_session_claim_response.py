"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict

from .managed_session_failure import ManagedSessionFailure
from .managed_session_ownership import ManagedSessionOwnership
from .managed_session_state import ManagedSessionState
from .model_base import Model
from .provision_response import ProvisionResponse


class ManagedSessionClaimResponse(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    error: ManagedSessionFailure | None = None
    ownership: ManagedSessionOwnership
    session: ProvisionResponse | None = None
    state: ManagedSessionState
