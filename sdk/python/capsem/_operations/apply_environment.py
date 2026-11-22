"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.environment_apply_request import EnvironmentApplyRequest
from ..models.environment_mutation_response import EnvironmentMutationResponse


async def apply_environment(
    transport: Transport,
    *,
    id: StrictStr,
    body: EnvironmentApplyRequest,
    request_timeout: float | None = None,
) -> EnvironmentMutationResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    payload = await transport.request(
        Method.POST, '/environments/{id}/apply',
        path_parameters={'id': id},
        body=body,
        json_body=True,
        timeout=request_timeout if request_timeout is not None else max(transport.timeout, 1020.0),
    )
    return TypeAdapter(EnvironmentMutationResponse).validate_json(payload)
