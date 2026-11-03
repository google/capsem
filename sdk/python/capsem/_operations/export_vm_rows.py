"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import MediaType, Method, Transport


async def export_vm_rows(
    transport: Transport,
    *,
    id: StrictStr,
    tables: StrictStr | None = None,
    request_timeout: float | None = None,
) -> bytes:
    id = TypeAdapter(StrictStr).validate_python(id)
    tables = TypeAdapter(StrictStr | None).validate_python(tables)
    return await transport.request(
        Method.GET, '/vms/{id}/export/rows.ndjson.gz',
        path_parameters={'id': id},
        query={'tables': tables},
        accept=MediaType.GZIP,
        timeout=request_timeout,
    )
