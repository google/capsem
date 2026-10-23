"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt, StrictStr

from .model_base import Model
from .security_ask_request import SecurityAskRequest


class SecurityAskListResponse(Model):
    generation: StrictStr
    pending_count: Annotated[StrictInt, Field(ge=0)]
    requests: list[SecurityAskRequest]
    vm_id: StrictStr
