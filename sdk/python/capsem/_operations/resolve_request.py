"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.environment_mutation_response import EnvironmentMutationResponse
from ..models.request_resolve_request import RequestResolveRequest


async def resolve_request(
    transport: Transport,
    *,
    id: StrictStr,
    body: RequestResolveRequest,
    request_timeout: float | None = None,
) -> EnvironmentMutationResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    payload = await transport.request(
        Method.POST, '/requests/{id}/resolve',
        path_parameters={'id': id},
        body=body,
        json_body=True,
        timeout=request_timeout,
    )
    return TypeAdapter(EnvironmentMutationResponse).validate_json(payload)
