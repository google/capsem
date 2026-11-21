"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import TypeAdapter

from .._transport import Method, Transport
from ..models.environment_plugin_list_response import EnvironmentPluginListResponse


async def list_new_environment_plugins(
    transport: Transport,
    *,
    request_timeout: float | None = None,
) -> EnvironmentPluginListResponse:
    payload = await transport.request(
        Method.GET, '/environments/plugins/list',
        timeout=request_timeout,
    )
    return TypeAdapter(EnvironmentPluginListResponse).validate_json(payload)
