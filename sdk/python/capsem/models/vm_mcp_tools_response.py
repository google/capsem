"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr

from .model_base import Model, UuidString
from .vm_mcp_tool import VmMcpTool


class VmMcpToolsResponse(Model):
    owner_generation: UuidString
    server_id: StrictStr
    tools: list[VmMcpTool]
    vm_id: StrictStr
