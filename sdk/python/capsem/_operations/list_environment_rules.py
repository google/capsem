"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.environment_rule_list_response import EnvironmentRuleListResponse
from ..models.model_base import UuidString


async def list_environment_rules(
    transport: Transport,
    *,
    id: StrictStr,
    revision_id: UuidString | None = None,
    request_timeout: float | None = None,
) -> EnvironmentRuleListResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    revision_id = TypeAdapter(UuidString | None).validate_python(revision_id)
    payload = await transport.request(
        Method.GET, '/environments/{id}/rules/list',
        path_parameters={'id': id},
        query={'revision_id': revision_id},
        timeout=request_timeout,
    )
    return TypeAdapter(EnvironmentRuleListResponse).validate_json(payload)
