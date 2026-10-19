"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt, StrictStr

from .error_code import ErrorCode
from .model_base import Model


class ErrorResponse(Model):
    code: ErrorCode | None = None
    error: StrictStr
    timeout_secs: Annotated[StrictInt, Field(ge=0)] | None = None
    vm_id: StrictStr | None = None
