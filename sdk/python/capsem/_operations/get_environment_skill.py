"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.environment_skill_detail_response import EnvironmentSkillDetailResponse
from ..models.model_base import UuidString


async def get_environment_skill(
    transport: Transport,
    *,
    id: StrictStr,
    skill_id: StrictStr,
    revision_id: UuidString | None = None,
    request_timeout: float | None = None,
) -> EnvironmentSkillDetailResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    skill_id = TypeAdapter(StrictStr).validate_python(skill_id)
    revision_id = TypeAdapter(UuidString | None).validate_python(revision_id)
    payload = await transport.request(
        Method.GET, '/environments/{id}/skills/{skill_id}/info',
        path_parameters={'id': id, 'skill_id': skill_id},
        query={'revision_id': revision_id},
        timeout=request_timeout,
    )
    return TypeAdapter(EnvironmentSkillDetailResponse).validate_json(payload)
