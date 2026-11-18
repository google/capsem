"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import ConfigDict, Field, StrictInt

from .model_base import Model
from .vm_lifecycle_selection import VmLifecycleSelection


class VmReadinessResponse(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    selection: VmLifecycleSelection
    startup_timeout_seconds: Annotated[StrictInt, Field(ge=0)]
