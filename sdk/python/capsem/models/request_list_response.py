"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictInt

from .model_base import Model
from .request_summary import RequestSummary


class RequestListResponse(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    next_cursor: StrictInt | None = None
    requests: list[RequestSummary]
