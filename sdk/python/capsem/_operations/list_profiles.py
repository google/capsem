"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import TypeAdapter

from .._transport import Method, Transport
from ..models.profiles_list_response import ProfilesListResponse


async def list_profiles(
    transport: Transport,
) -> ProfilesListResponse:
    payload = await transport.request(
        Method.GET, '/profiles/list',
    )
    return TypeAdapter(ProfilesListResponse).validate_json(payload)
