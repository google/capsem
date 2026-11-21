"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.environment_toml_validation_request import EnvironmentTomlValidationRequest
from ..models.environment_toml_validation_response import EnvironmentTomlValidationResponse


async def validate_environment_toml(
    transport: Transport,
    *,
    id: StrictStr,
    revision_id: StrictStr,
    body: EnvironmentTomlValidationRequest,
    request_timeout: float | None = None,
) -> EnvironmentTomlValidationResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    revision_id = TypeAdapter(StrictStr).validate_python(revision_id)
    payload = await transport.request(
        Method.POST, '/environments/{id}/revisions/{revision_id}/validate',
        path_parameters={'id': id, 'revision_id': revision_id},
        body=body,
        json_body=True,
        timeout=request_timeout,
    )
    return TypeAdapter(EnvironmentTomlValidationResponse).validate_json(payload)
