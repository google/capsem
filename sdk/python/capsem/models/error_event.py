"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr

from .model_base import Model


class ErrorEvent(Model):
    binary: StrictStr
    level: StrictStr
    message: StrictStr
    target: StrictStr | None = None
    ts: StrictStr
