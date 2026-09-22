"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr

from .model_base import Model


class SupplyChainManifestEvidence(Model):
    blake3: StrictStr | None = None
    origin: StrictStr | None = None
    path: StrictStr
    source: StrictStr | None = None
