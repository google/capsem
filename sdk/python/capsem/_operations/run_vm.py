"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import TypeAdapter

from .._transport import Method, Transport
from ..models.exec_response import ExecResponse
from ..models.run_request import RunRequest


async def run_vm(
    transport: Transport,
    *,
    body: RunRequest,
    request_timeout: float | None = None,
) -> ExecResponse:
    payload = await transport.request(
        Method.POST, '/run',
        body=body,
        json_body=True,
        timeout=request_timeout,
    )
    return TypeAdapter(ExecResponse).validate_json(payload)
