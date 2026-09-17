"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.exposure_list_response import ExposureListResponse


async def list_vm_exposures(
    transport: Transport,
    *,
    id: StrictStr,
    request_timeout: float | None = None,
) -> ExposureListResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    payload = await transport.request(
        Method.GET, '/vms/{id}/exposures',
        path_parameters={'id': id},
        timeout=request_timeout,
    )
    return TypeAdapter(ExposureListResponse).validate_json(payload)
