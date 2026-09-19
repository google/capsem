"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictStr

from .model_base import Model
from .raw_content_kind import RawContentKind
from .raw_content_reason import RawContentReason


class RawContent(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    kind: RawContentKind
    raw: StrictStr
    reason: RawContentReason
