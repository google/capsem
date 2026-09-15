"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.exposure_info import ExposureInfo
from ..models.exposure_request import ExposureRequest


async def create_vm_exposure(
    transport: Transport,
    *,
    id: StrictStr,
    body: ExposureRequest,
) -> ExposureInfo:
    id = TypeAdapter(StrictStr).validate_python(id)
    payload = await transport.request(
        Method.POST, '/vms/{id}/exposures',
        path_parameters={'id': id},
        body=body,
        json_body=True,
    )
    return TypeAdapter(ExposureInfo).validate_json(payload)
