"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictBool, StrictStr

from .model_base import Model


class PersistResponse(Model):
    name: StrictStr
    success: StrictBool
