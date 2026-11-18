"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictBool, StrictStr

from .environment_revision_reference import EnvironmentRevisionReference
from .model_base import Model, UuidString


class VmLifecycleSelection(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    environment: EnvironmentRevisionReference | None = None
    generation: UuidString | None = None
    id: StrictStr
    running: StrictBool
