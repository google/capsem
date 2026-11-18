"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import ConfigDict, Field, StrictInt, StrictStr

from .model_base import Model


class EnvironmentIdentity(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    id: StrictStr
    parent_environment_id: StrictStr | None = None
    parent_revision_id: StrictStr | None = None
    revision_id: StrictStr
    schema_version: Annotated[StrictInt, Field(ge=0)]
