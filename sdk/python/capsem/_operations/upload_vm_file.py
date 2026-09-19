"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictBool, StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.upload_response import UploadResponse


async def upload_vm_file(
    transport: Transport,
    *,
    id: StrictStr,
    path: StrictStr,
    exact: StrictBool | None = None,
    body: bytes,
    request_timeout: float | None = None,
) -> UploadResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    path = TypeAdapter(StrictStr).validate_python(path)
    exact = TypeAdapter(StrictBool | None).validate_python(exact)
    payload = await transport.request(
        Method.POST, '/vms/{id}/files/content',
        path_parameters={'id': id},
        query={'path': path, 'exact': exact},
        body=body,
        timeout=request_timeout,
    )
    return TypeAdapter(UploadResponse).validate_json(payload)
