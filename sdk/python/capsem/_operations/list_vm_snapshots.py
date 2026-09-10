"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.snapshots_list import SnapshotsList


async def list_vm_snapshots(
    transport: Transport,
    *,
    id: StrictStr,
) -> SnapshotsList:
    id = TypeAdapter(StrictStr).validate_python(id)
    payload = await transport.request(
        Method.GET, '/vms/{id}/snapshots/list',
        path_parameters={'id': id},
    )
    return TypeAdapter(SnapshotsList).validate_json(payload)
