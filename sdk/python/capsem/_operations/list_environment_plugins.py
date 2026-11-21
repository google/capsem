"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.environment_plugin_list_response import EnvironmentPluginListResponse
from ..models.model_base import UuidString


async def list_environment_plugins(
    transport: Transport,
    *,
    id: StrictStr,
    revision_id: UuidString | None = None,
    request_timeout: float | None = None,
) -> EnvironmentPluginListResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    revision_id = TypeAdapter(UuidString | None).validate_python(revision_id)
    payload = await transport.request(
        Method.GET, '/environments/{id}/plugins/list',
        path_parameters={'id': id},
        query={'revision_id': revision_id},
        timeout=request_timeout,
    )
    return TypeAdapter(EnvironmentPluginListResponse).validate_json(payload)
