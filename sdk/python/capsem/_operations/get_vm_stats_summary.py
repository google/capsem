"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.vm_stats_summary_response import VmStatsSummaryResponse


async def get_vm_stats_summary(
    transport: Transport,
    *,
    id: StrictStr,
) -> VmStatsSummaryResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    payload = await transport.request(
        Method.GET, '/vms/{id}/stats/summary',
        path_parameters={'id': id},
    )
    return TypeAdapter(VmStatsSummaryResponse).validate_json(payload)
