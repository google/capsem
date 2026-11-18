"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictStr

from .environment_revision_reference import EnvironmentRevisionReference
from .model_base import Model, UuidString
from .request_application_state import RequestApplicationState
from .request_state import RequestState


class EnvironmentMutationResponse(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    application: RequestApplicationState
    application_error: StrictStr | None = None
    decision: RequestState
    environment_id: UuidString
    request_id: UuidString
    result: EnvironmentRevisionReference | None = None
