"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt, StrictStr, TypeAdapter

from .._transport import Method, Transport
from ..models.rule_catalog_list_response import RuleCatalogListResponse


async def list_catalog_rules(
    transport: Transport,
    *,
    search: StrictStr | None = None,
    after: StrictStr | None = None,
    limit: Annotated[StrictInt, Field(ge=0)] | None = None,
    request_timeout: float | None = None,
) -> RuleCatalogListResponse:
    search = TypeAdapter(StrictStr | None).validate_python(search)
    after = TypeAdapter(StrictStr | None).validate_python(after)
    limit = TypeAdapter(Annotated[StrictInt, Field(ge=0)] | None).validate_python(limit)
    payload = await transport.request(
        Method.GET, '/rules/list',
        query={'search': search, 'after': after, 'limit': limit},
        timeout=request_timeout,
    )
    return TypeAdapter(RuleCatalogListResponse).validate_json(payload)
