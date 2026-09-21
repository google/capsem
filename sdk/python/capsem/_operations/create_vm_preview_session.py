"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.preview_session_response import PreviewSessionResponse


async def create_vm_preview_session(
    transport: Transport,
    *,
    id: StrictStr,
    exposure_id: StrictStr,
    request_timeout: float | None = None,
) -> PreviewSessionResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    exposure_id = TypeAdapter(StrictStr).validate_python(exposure_id)
    payload = await transport.request(
        Method.POST, '/vms/{id}/exposures/{exposure_id}/preview-session',
        path_parameters={'id': id, 'exposure_id': exposure_id},
        timeout=request_timeout,
    )
    return TypeAdapter(PreviewSessionResponse).validate_json(payload)
