"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictStr

from .environment_skill_detail_response import EnvironmentSkillDetailResponse
from .model_base import Model, UuidString


class VmSkillDetailResponse(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    owner_generation: UuidString
    skill: EnvironmentSkillDetailResponse
    vm_id: StrictStr
