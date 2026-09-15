"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr

from .model_base import Model


class SupplyChainReference(Model):
    format: StrictStr | None = None
    generator: StrictStr | None = None
    name: StrictStr
    release_artifact: StrictStr | None = None
    route: StrictStr | None = None
    scope: StrictStr | None = None
    workflow: StrictStr | None = None
