"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import TypeAlias

from .json_content import JsonContent
from .raw_content import RawContent
from .text_content import TextContent

CapturedContent: TypeAlias = JsonContent | TextContent | RawContent
