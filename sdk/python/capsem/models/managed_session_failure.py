"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictStr

from .managed_session_error_code import ManagedSessionErrorCode
from .model_base import Model


class ManagedSessionFailure(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    code: ManagedSessionErrorCode
    error: StrictStr
