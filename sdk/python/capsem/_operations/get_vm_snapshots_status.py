"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.snapshots_status import SnapshotsStatus


async def get_vm_snapshots_status(
    transport: Transport,
    *,
    id: StrictStr,
) -> SnapshotsStatus:
    id = TypeAdapter(StrictStr).validate_python(id)
    payload = await transport.request(
        Method.GET, '/vms/{id}/snapshots/status',
        path_parameters={'id': id},
    )
    return TypeAdapter(SnapshotsStatus).validate_json(payload)
