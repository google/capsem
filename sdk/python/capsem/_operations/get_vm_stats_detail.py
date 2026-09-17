"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.vm_stats_detail_response import VmStatsDetailResponse


async def get_vm_stats_detail(
    transport: Transport,
    *,
    id: StrictStr,
    request_timeout: float | None = None,
) -> VmStatsDetailResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    payload = await transport.request(
        Method.GET, '/vms/{id}/stats/detail',
        path_parameters={'id': id},
        timeout=request_timeout,
    )
    return TypeAdapter(VmStatsDetailResponse).validate_json(payload)
