"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.environment_apply_preview_request import EnvironmentApplyPreviewRequest
from ..models.environment_apply_preview_response import EnvironmentApplyPreviewResponse


async def preview_environment_application(
    transport: Transport,
    *,
    id: StrictStr,
    body: EnvironmentApplyPreviewRequest,
    request_timeout: float | None = None,
) -> EnvironmentApplyPreviewResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    payload = await transport.request(
        Method.POST, '/environments/{id}/apply/preview',
        path_parameters={'id': id},
        body=body,
        json_body=True,
        timeout=request_timeout,
    )
    return TypeAdapter(EnvironmentApplyPreviewResponse).validate_json(payload)
