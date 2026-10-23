"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.browser_acknowledge_request import BrowserAcknowledgeRequest
from ..models.browser_acknowledge_response import BrowserAcknowledgeResponse


async def acknowledge_browser_request(
    transport: Transport,
    *,
    id: StrictStr,
    body: BrowserAcknowledgeRequest,
    request_timeout: float | None = None,
) -> BrowserAcknowledgeResponse:
    id = TypeAdapter(StrictStr).validate_python(id)
    payload = await transport.request(
        Method.POST, '/vms/{id}/browser/acknowledge',
        path_parameters={'id': id},
        body=body,
        json_body=True,
        timeout=request_timeout,
    )
    return TypeAdapter(BrowserAcknowledgeResponse).validate_json(payload)
