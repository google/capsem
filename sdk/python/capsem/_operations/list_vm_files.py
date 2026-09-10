"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictInt, StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.file_list_response import FileListResponse


async def list_vm_files(
    transport: Transport,
    *,
    id: StrictStr,
    path: StrictStr | None = None,
    depth: StrictInt | None = None,
) -> FileListResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    path = TypeAdapter(StrictStr | None).validate_python(path)
    depth = TypeAdapter(StrictInt | None).validate_python(depth)
    payload = await transport.request(
        Method.GET, '/vms/{id}/files/list',
        path_parameters={'id': id},
        query={'path': path, 'depth': depth},
    )
    return TypeAdapter(FileListResponse).validate_json(payload)
