"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictStr

from .environment_list_item import EnvironmentListItem
from .model_base import Model


class EnvironmentListResponse(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    environments: list[EnvironmentListItem]
    next_cursor: StrictStr | None = None
