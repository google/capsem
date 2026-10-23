"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr

from .image_package_inventory import ImagePackageInventory
from .model_base import Model


class ImageInventoryResponse(Model):
    architecture: StrictStr
    artifact_reference: StrictStr | None = None
    image: StrictStr
    inventory: ImagePackageInventory | None = None
