"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import MediaType, Method, Transport


async def export_vm_bodies(
    transport: Transport,
    *,
    id: StrictStr,
    request_timeout: float | None = None,
) -> bytes:
    id = TypeAdapter(StrictStr).validate_python(id)
    return await transport.request(
        Method.GET, '/vms/{id}/bodies/export.warc.gz',
        path_parameters={'id': id},
        accept=MediaType.GZIP,
        timeout=request_timeout,
    )
