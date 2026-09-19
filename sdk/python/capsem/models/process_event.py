"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt, StrictStr

from .exec_source import ExecSource
from .model_base import Model


class ProcessEvent(Model):
    command: StrictStr
    credential_ref: StrictStr | None = None
    duration_ms: Annotated[StrictInt, Field(ge=0)] | None = None
    event_id: StrictStr
    exec_id: Annotated[StrictInt, Field(ge=0)]
    exit_code: StrictInt | None = None
    pid: Annotated[StrictInt, Field(ge=0)] | None = None
    process_name: StrictStr | None = None
    source: ExecSource
    stderr_bytes: Annotated[StrictInt, Field(ge=0)] | None = None
    stdout_bytes: Annotated[StrictInt, Field(ge=0)] | None = None
    timestamp: StrictStr
    trace_id: StrictStr | None = None
