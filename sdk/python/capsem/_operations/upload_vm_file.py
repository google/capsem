"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.upload_response import UploadResponse


async def upload_vm_file(
    transport: Transport,
    *,
    id: StrictStr,
    path: StrictStr,
    body: bytes,
) -> UploadResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    path = TypeAdapter(StrictStr).validate_python(path)
    payload = await transport.request(
        Method.POST, '/vms/{id}/files/content',
        path_parameters={'id': id},
        query={'path': path},
        body=body,
    )
    return TypeAdapter(UploadResponse).validate_json(payload)
