"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.mcp_catalog_detail_response import McpCatalogDetailResponse


async def get_catalog_mcp(
    transport: Transport,
    *,
    mcp_id: StrictStr,
    version: StrictStr,
    snapshot: StrictStr,
    request_timeout: float | None = None,
) -> McpCatalogDetailResponse:
    mcp_id = TypeAdapter(StrictStr).validate_python(mcp_id)
    version = TypeAdapter(StrictStr).validate_python(version)
    snapshot = TypeAdapter(StrictStr).validate_python(snapshot)
    payload = await transport.request(
        Method.GET, '/mcps/{mcp_id}/info',
        path_parameters={'mcp_id': mcp_id},
        query={'version': version, 'snapshot': snapshot},
        timeout=request_timeout,
    )
    return TypeAdapter(McpCatalogDetailResponse).validate_json(payload)
