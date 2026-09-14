"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictInt, StrictStr

from .model_base import Model
from .network_member_info import NetworkMemberInfo


class NetworkInfo(Model):
    created_unix_ms: StrictInt
    id: StrictStr
    members: list[NetworkMemberInfo]
    name: StrictStr
