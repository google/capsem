"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt, StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.model_base import UuidString
from ..models.vm_skill_list_response import VmSkillListResponse


async def list_vm_skills(
    transport: Transport,
    *,
    id: StrictStr,
    revision_id: UuidString | None = None,
    search: StrictStr | None = None,
    after: StrictStr | None = None,
    limit: Annotated[StrictInt, Field(ge=0)] | None = None,
    request_timeout: float | None = None,
) -> VmSkillListResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    revision_id = TypeAdapter(UuidString | None).validate_python(revision_id)
    search = TypeAdapter(StrictStr | None).validate_python(search)
    after = TypeAdapter(StrictStr | None).validate_python(after)
    limit = TypeAdapter(Annotated[StrictInt, Field(ge=0)] | None).validate_python(limit)
    payload = await transport.request(
        Method.GET, '/vms/{id}/skills/list',
        path_parameters={'id': id},
        query={'revision_id': revision_id, 'search': search, 'after': after, 'limit': limit},
        timeout=request_timeout,
    )
    return TypeAdapter(VmSkillListResponse).validate_json(payload)
