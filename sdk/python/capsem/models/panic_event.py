"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr

from .model_base import Model


class PanicEvent(Model):
    binary: StrictStr
    frames: list[StrictStr]
    location: StrictStr | None = None
    message: StrictStr
    thread: StrictStr | None = None
    ts: StrictStr
