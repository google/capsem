"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictInt, StrictStr

from .environment_sandbox_target import EnvironmentSandboxTarget
from .model_base import Model
from .request_application_composition import RequestApplicationComposition
from .request_application_state import RequestApplicationState


class RequestTargetApplication(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    composition: RequestApplicationComposition | None = None
    failure_reason: StrictStr | None = None
    state: RequestApplicationState
    target: EnvironmentSandboxTarget
    timestamp_unix_ms: StrictInt | None = None
