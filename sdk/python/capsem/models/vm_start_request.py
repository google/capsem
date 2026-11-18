"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict

from .model_base import Model
from .vm_lifecycle_selection import VmLifecycleSelection


class VmStartRequest(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    expected: VmLifecycleSelection | None = None
