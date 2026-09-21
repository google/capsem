"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt, StrictStr

from .model_base import Model
from .profile_availability_summary import ProfileAvailabilitySummary
from .profile_update_semantics import ProfileUpdateSemantics


class ProfileSummary(Model):
    availability: ProfileAvailabilitySummary
    default_rule_count: Annotated[StrictInt, Field(ge=0)]
    description: StrictStr
    icon_svg: StrictStr | None = None
    id: StrictStr
    mcp_server_count: Annotated[StrictInt, Field(ge=0)]
    name: StrictStr
    plugin_count: Annotated[StrictInt, Field(ge=0)]
    rule_count: Annotated[StrictInt, Field(ge=0)]
    source: StrictStr
    update_semantics: ProfileUpdateSemantics
