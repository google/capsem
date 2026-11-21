"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import ConfigDict, Field, StrictInt

from .environment_list_summary import EnvironmentListSummary
from .environment_revision_reference import EnvironmentRevisionReference
from .model_base import Model


class EnvironmentListItem(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    environment: EnvironmentRevisionReference
    instance_count: Annotated[StrictInt, Field(ge=0)] | None = None
    summary: EnvironmentListSummary | None = None
