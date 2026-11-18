"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.vm_lifecycle_intent import VmLifecycleIntent
from ..models.vm_readiness_response import VmReadinessResponse


async def get_vm_readiness(
    transport: Transport,
    *,
    id: StrictStr,
    intent: VmLifecycleIntent,
    request_timeout: float | None = None,
) -> VmReadinessResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    intent = TypeAdapter(VmLifecycleIntent).validate_python(intent)
    payload = await transport.request(
        Method.GET, '/vms/{id}/readiness',
        path_parameters={'id': id},
        query={'intent': intent},
        timeout=request_timeout,
    )
    return TypeAdapter(VmReadinessResponse).validate_json(payload)
