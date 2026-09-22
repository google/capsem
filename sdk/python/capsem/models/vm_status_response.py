"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictBool, StrictInt, StrictStr

from .model_base import Model
from .storage_diagnostics import StorageDiagnostics
from .vm_action import VmAction
from .vm_lifecycle_state import VmLifecycleState


class VmStatusResponse(Model):
    nonnullable_optional = frozenset(['can_resume', 'persistent'])
    available_actions: list[VmAction]
    can_resume: StrictBool | None = None
    created_at: StrictStr | None = None
    id: StrictStr
    last_error: StrictStr | None = None
    name: StrictStr
    persistent: StrictBool | None = None
    pid: Annotated[StrictInt, Field(ge=0)] | None = None
    resume_blocked_reason: StrictStr | None = None
    status: VmLifecycleState
    storage: StorageDiagnostics | None = None
    uptime_secs: Annotated[StrictInt, Field(ge=0)] | None = None
