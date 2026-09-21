"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr

from .mcp_permission_action import McpPermissionAction
from .model_base import Model


class McpDefaultPermissionResponse(Model):
    action: McpPermissionAction
    rule_id: StrictStr | None = None
    source: StrictStr
