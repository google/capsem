"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.skill_catalog_detail_response import SkillCatalogDetailResponse


async def get_catalog_skill(
    transport: Transport,
    *,
    skill_id: StrictStr,
    version: StrictStr,
    snapshot: StrictStr,
    request_timeout: float | None = None,
) -> SkillCatalogDetailResponse:
    skill_id = TypeAdapter(StrictStr).validate_python(skill_id)
    version = TypeAdapter(StrictStr).validate_python(version)
    snapshot = TypeAdapter(StrictStr).validate_python(snapshot)
    payload = await transport.request(
        Method.GET, '/skills/{skill_id}/info',
        path_parameters={'skill_id': skill_id},
        query={'version': version, 'snapshot': snapshot},
        timeout=request_timeout,
    )
    return TypeAdapter(SkillCatalogDetailResponse).validate_json(payload)
