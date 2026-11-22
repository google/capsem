"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictBool, StrictStr

from .environment_revision_reference import EnvironmentRevisionReference
from .environment_skill_summary import EnvironmentSkillSummary
from .model_base import Model, UuidString


class VmSkillListResponse(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    discovery_fresh: StrictBool
    next_cursor: StrictStr | None = None
    owner_generation: UuidString
    revision: EnvironmentRevisionReference | None = None
    skills: list[EnvironmentSkillSummary]
    vm_id: StrictStr
