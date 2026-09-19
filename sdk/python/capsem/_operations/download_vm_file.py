"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictBool, StrictStr, TypeAdapter

from .._transport import MediaType, Method, Transport


async def download_vm_file(
    transport: Transport,
    *,
    id: StrictStr,
    path: StrictStr,
    exact: StrictBool | None = None,
    request_timeout: float | None = None,
) -> bytes:
    id = TypeAdapter(StrictStr).validate_python(id)
    path = TypeAdapter(StrictStr).validate_python(path)
    exact = TypeAdapter(StrictBool | None).validate_python(exact)
    return await transport.request(
        Method.GET, '/vms/{id}/files/content',
        path_parameters={'id': id},
        query={'path': path, 'exact': exact},
        accept=MediaType.BINARY,
        timeout=request_timeout,
    )
