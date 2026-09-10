"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import MediaType, Method, Transport


async def download_vm_file(
    transport: Transport,
    *,
    id: StrictStr,
    path: StrictStr,
) -> bytes:
    id = TypeAdapter(StrictStr).validate_python(id)
    path = TypeAdapter(StrictStr).validate_python(path)
    return await transport.request(
        Method.GET, '/vms/{id}/files/content',
        path_parameters={'id': id},
        query={'path': path},
        accept=MediaType.BINARY,
    )
