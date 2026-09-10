"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictBool, StrictFloat, StrictInt, StrictStr

from .model_base import Model
from .vm_action import VmAction
from .vm_lifecycle_state import VmLifecycleState


class VmSummary(Model):
    nonnullable_optional = frozenset(['can_resume'])
    allowed_requests: Annotated[StrictInt, Field(ge=0)] | None = None
    available_actions: list[VmAction]
    can_resume: StrictBool | None = None
    denied_requests: Annotated[StrictInt, Field(ge=0)] | None = None
    id: StrictStr
    last_error: StrictStr | None = None
    model_call_count: Annotated[StrictInt, Field(ge=0)] | None = None
    name: StrictStr | None = None
    persistent: StrictBool
    profile_id: StrictStr
    resume_blocked_reason: StrictStr | None = None
    status: VmLifecycleState
    total_estimated_cost: StrictFloat | None = None
    total_file_events: Annotated[StrictInt, Field(ge=0)] | None = None
    total_input_tokens: Annotated[StrictInt, Field(ge=0)] | None = None
    total_output_tokens: Annotated[StrictInt, Field(ge=0)] | None = None
    total_requests: Annotated[StrictInt, Field(ge=0)] | None = None
    total_tool_calls: Annotated[StrictInt, Field(ge=0)] | None = None
    uptime_secs: Annotated[StrictInt, Field(ge=0)] | None = None
