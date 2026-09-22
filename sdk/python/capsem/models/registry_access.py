"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr

from .model_base import Model


class RegistryAccess(Model):
    ca_pem: StrictStr | None = None
    password: StrictStr | None = None
    username: StrictStr | None = None
