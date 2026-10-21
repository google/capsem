"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Literal

from pydantic import StrictStr

from .model_base import Model


class StreamStatusError(Model):
    message: StrictStr
    type: Literal["error"]
