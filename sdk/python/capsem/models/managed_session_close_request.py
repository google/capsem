"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import ConfigDict, Field, StrictStr

from .managed_session_outcome import ManagedSessionOutcome
from .model_base import Model


class ManagedSessionCloseRequest(Model):
    private_input = True
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid", hide_input_in_errors=True)
    outcome: ManagedSessionOutcome
    ownership_token: Annotated[StrictStr, Field(pattern='^[0-9a-f]{64}$', min_length=64, max_length=64)] = Field(repr=False)
