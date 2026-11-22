"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictStr

from .model_base import Model


class CatalogSnapshot(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    digest: StrictStr
    reference: StrictStr
    snapshot: StrictStr
