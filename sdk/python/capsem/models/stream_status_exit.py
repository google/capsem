"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Literal

from pydantic import StrictBool, StrictInt

from .model_base import Model


class StreamStatusExit(Model):
    code: StrictInt
    truncated: StrictBool
    type: Literal["exit"]
