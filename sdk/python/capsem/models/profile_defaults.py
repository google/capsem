"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr

from .model_base import Model


class ProfileDefaults(Model):
    container: StrictStr | None = None
    vm: StrictStr | None = None
