"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict

from .json_content_kind import JsonContentKind
from .model_base import JsonValue, Model


class JsonContent(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    kind: JsonContentKind
    value: JsonValue
