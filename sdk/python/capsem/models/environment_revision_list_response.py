"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictInt

from .environment_revision_summary import EnvironmentRevisionSummary
from .model_base import Model


class EnvironmentRevisionListResponse(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    next_cursor: StrictInt | None = None
    revisions: list[EnvironmentRevisionSummary]
