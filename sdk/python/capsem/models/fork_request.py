"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr

from .model_base import Model


class ForkRequest(Model):
    description: StrictStr | None = None
    labels: dict[str, StrictStr] | None = None
    name: StrictStr
