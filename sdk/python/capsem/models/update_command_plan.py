"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr

from .model_base import Model


class UpdateCommandPlan(Model):
    args: list[StrictStr]
    program: StrictStr
