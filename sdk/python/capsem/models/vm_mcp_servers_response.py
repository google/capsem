"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr

from .model_base import Model, UuidString
from .vm_mcp_server import VmMcpServer


class VmMcpServersResponse(Model):
    owner_generation: UuidString
    servers: list[VmMcpServer]
    vm_id: StrictStr
