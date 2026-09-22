"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from ipaddress import IPv4Address

from pydantic import StrictInt, StrictStr

from .model_base import Model
from .network_member_state import NetworkMemberState


class NetworkMemberInfo(Model):
    address: IPv4Address
    state: NetworkMemberState
    updated_unix_ms: StrictInt
    vm_id: StrictStr
