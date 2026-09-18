"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.preview_sessions_revoked_response import PreviewSessionsRevokedResponse


async def revoke_vm_preview_sessions(
    transport: Transport,
    *,
    id: StrictStr,
    exposure_id: StrictStr,
    request_timeout: float | None = None,
) -> PreviewSessionsRevokedResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    exposure_id = TypeAdapter(StrictStr).validate_python(exposure_id)
    payload = await transport.request(
        Method.DELETE, '/vms/{id}/exposures/{exposure_id}/preview-session',
        path_parameters={'id': id, 'exposure_id': exposure_id},
        timeout=request_timeout,
    )
    return TypeAdapter(PreviewSessionsRevokedResponse).validate_json(payload)
