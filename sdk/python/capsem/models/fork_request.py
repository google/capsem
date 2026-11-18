"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictStr

from .model_base import Model
from .vm_lifecycle_selection import VmLifecycleSelection


class ForkRequest(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    description: StrictStr | None = None
    expected: VmLifecycleSelection | None = None
    labels: dict[str, StrictStr] | None = None
    name: StrictStr
