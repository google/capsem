"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictBool, StrictFloat, StrictInt, StrictStr

from .model_base import Model
from .session_db_status import SessionDbStatus
from .storage_diagnostics import StorageDiagnostics
from .vm_action import VmAction
from .vm_ai_info import VmAiInfo
from .vm_files_info import VmFilesInfo
from .vm_lifecycle_state import VmLifecycleState
from .vm_network_info import VmNetworkInfo


class SandboxInfo(Model):
    nonnullable_optional = frozenset(['can_resume', 'persistent'])
    ai: VmAiInfo | None = None
    allowed_requests: Annotated[StrictInt, Field(ge=0)] | None = None
    available_actions: list[VmAction]
    can_resume: StrictBool | None = None
    cpus: Annotated[StrictInt, Field(ge=0)] | None = None
    created_at: StrictStr | None = None
    denied_requests: Annotated[StrictInt, Field(ge=0)] | None = None
    description: StrictStr | None = None
    files: VmFilesInfo | None = None
    forked_from: StrictStr | None = None
    id: StrictStr
    last_error: StrictStr | None = None
    model_call_count: Annotated[StrictInt, Field(ge=0)] | None = None
    name: StrictStr | None = None
    network: VmNetworkInfo | None = None
    persistent: StrictBool | None = None
    pid: Annotated[StrictInt, Field(ge=0)]
    profile_id: StrictStr
    ram_mb: Annotated[StrictInt, Field(ge=0)] | None = None
    resume_blocked_reason: StrictStr | None = None
    session_db: SessionDbStatus | None = None
    size_bytes: Annotated[StrictInt, Field(ge=0)] | None = None
    status: VmLifecycleState
    storage: StorageDiagnostics | None = None
    total_estimated_cost: StrictFloat | None = None
    total_file_events: Annotated[StrictInt, Field(ge=0)] | None = None
    total_input_tokens: Annotated[StrictInt, Field(ge=0)] | None = None
    total_output_tokens: Annotated[StrictInt, Field(ge=0)] | None = None
    total_requests: Annotated[StrictInt, Field(ge=0)] | None = None
    total_thinking_tokens: Annotated[StrictInt, Field(ge=0)] | None = None
    total_tool_calls: Annotated[StrictInt, Field(ge=0)] | None = None
    uptime_secs: Annotated[StrictInt, Field(ge=0)] | None = None
    version: StrictStr | None = None
