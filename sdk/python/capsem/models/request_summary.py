"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictInt, StrictStr

from .environment_revision_reference import EnvironmentRevisionReference
from .environment_sandbox_target import EnvironmentSandboxTarget
from .model_base import Model, UuidString
from .request_application_state import RequestApplicationState
from .request_kind import RequestKind
from .request_state import RequestState


class RequestSummary(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    actor: StrictStr
    application: RequestApplicationState
    base: EnvironmentRevisionReference | None = None
    decision: RequestState
    environment_id: UuidString
    expires_at_unix_ms: StrictInt
    kind: RequestKind
    reason: StrictStr
    request_id: UuidString
    requester: EnvironmentSandboxTarget | None = None
    result: EnvironmentRevisionReference | None = None
    source: EnvironmentRevisionReference | None = None
    timestamp_unix_ms: StrictInt
