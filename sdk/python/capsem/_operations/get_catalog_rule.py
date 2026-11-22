"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.rule_catalog_detail_response import RuleCatalogDetailResponse


async def get_catalog_rule(
    transport: Transport,
    *,
    rule_id: StrictStr,
    version: StrictStr,
    snapshot: StrictStr,
    request_timeout: float | None = None,
) -> RuleCatalogDetailResponse:
    rule_id = TypeAdapter(StrictStr).validate_python(rule_id)
    version = TypeAdapter(StrictStr).validate_python(version)
    snapshot = TypeAdapter(StrictStr).validate_python(snapshot)
    payload = await transport.request(
        Method.GET, '/rules/{rule_id}/info',
        path_parameters={'rule_id': rule_id},
        query={'version': version, 'snapshot': snapshot},
        timeout=request_timeout,
    )
    return TypeAdapter(RuleCatalogDetailResponse).validate_json(payload)
