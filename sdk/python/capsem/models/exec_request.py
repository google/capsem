"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt, StrictStr

from .exec_target import ExecTarget
from .model_base import Model


class ExecRequest(Model):
    command: StrictStr
    target: ExecTarget | None = None
    timeout_secs: Annotated[StrictInt, Field(ge=0)] | None = None
