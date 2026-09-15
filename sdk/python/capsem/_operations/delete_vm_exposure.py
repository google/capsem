"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.vm_action_response import VmActionResponse


async def delete_vm_exposure(
    transport: Transport,
    *,
    id: StrictStr,
    exposure_id: StrictStr,
) -> VmActionResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    exposure_id = TypeAdapter(StrictStr).validate_python(exposure_id)
    payload = await transport.request(
        Method.DELETE, '/vms/{id}/exposures/{exposure_id}',
        path_parameters={'id': id, 'exposure_id': exposure_id},
    )
    return TypeAdapter(VmActionResponse).validate_json(payload)
