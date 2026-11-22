"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictBool, StrictInt, StrictStr

from .model_base import Model
from .vm_mcp_server_kind import VmMcpServerKind


class VmMcpServer(Model):
    connected: StrictBool
    enabled: StrictBool
    kind: VmMcpServerKind
    name: StrictStr
    tool_count: Annotated[StrictInt, Field(ge=0)]
