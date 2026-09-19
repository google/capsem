"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from .model_base import Model
from .panic_event import PanicEvent


class PanicsResponse(Model):
    panics: list[PanicEvent]
