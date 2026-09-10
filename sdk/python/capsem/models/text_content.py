"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictStr

from .model_base import Model
from .text_content_kind import TextContentKind


class TextContent(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    kind: TextContentKind
    text: StrictStr
