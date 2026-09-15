"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictBool, StrictStr

from .model_base import Model
from .vm_action import VmAction
from .vm_lifecycle_state import VmLifecycleState


class ProvisionResponse(Model):
    nonnullable_optional = frozenset(['can_resume', 'persistent'])
    available_actions: list[VmAction]
    can_resume: StrictBool | None = None
    id: StrictStr
    name: StrictStr
    persistent: StrictBool | None = None
    profile_id: StrictStr
    status: VmLifecycleState
    uds_path: StrictStr | None = None
