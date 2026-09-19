"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictBool, StrictStr

from .mcp_permission_action import McpPermissionAction
from .model_base import JsonValue, Model


class McpToolInfoResponse(Model):
    annotations: JsonValue = None
    description: StrictStr | None = None
    namespaced_name: StrictStr
    original_name: StrictStr
    permission_action: McpPermissionAction
    permission_source: StrictStr
    pin_changed: StrictBool
    pin_hash: StrictStr | None = None
    server_name: StrictStr
