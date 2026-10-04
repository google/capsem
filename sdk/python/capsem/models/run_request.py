"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import ConfigDict, Field, StrictInt, StrictStr

from .model_base import Model


class RunRequest(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    command: StrictStr
    cpus: Annotated[StrictInt, Field(ge=0)] | None = None
    env: dict[str, StrictStr] | None = None
    ram_mb: Annotated[StrictInt, Field(ge=0)] | None = None
    timeout_secs: Annotated[StrictInt, Field(ge=0)] | None = None
