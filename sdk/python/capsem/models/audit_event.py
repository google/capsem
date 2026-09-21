"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt, StrictStr

from .model_base import Model


class AuditEvent(Model):
    argv: StrictStr
    audit_id: StrictStr | None = None
    comm: StrictStr | None = None
    credential_ref: StrictStr | None = None
    cwd: StrictStr | None = None
    event_id: StrictStr
    exe: StrictStr
    exec_event_id: StrictInt | None = None
    exit_code: StrictInt | None = None
    parent_exe: StrictStr | None = None
    pid: Annotated[StrictInt, Field(ge=0)]
    ppid: Annotated[StrictInt, Field(ge=0)]
    session_id: Annotated[StrictInt, Field(ge=0)] | None = None
    timestamp: StrictStr
    trace_id: StrictStr | None = None
    tty: StrictStr | None = None
    uid: Annotated[StrictInt, Field(ge=0)]
