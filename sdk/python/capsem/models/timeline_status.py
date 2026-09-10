"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import TypeAlias

from pydantic import StrictInt

from .tool_decision import ToolDecision

TimelineStatus: TypeAlias = StrictInt | ToolDecision
