"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr

from .model_base import JsonValue, Model


class VmMcpTool(Model):
    annotations: JsonValue = None
    definition_digest: StrictStr
    description: StrictStr | None = None
    input_schema: JsonValue
    namespaced_name: StrictStr
    original_name: StrictStr
