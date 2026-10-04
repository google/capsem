"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr

from .model_base import Model


class CatalogInfo(Model):
    channel: StrictStr
    digest: StrictStr
    generated_at: StrictStr | None = None
    reference: StrictStr
