"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr

from .image_package import ImagePackage
from .image_tool import ImageTool
from .model_base import Model


class ImagePackageInventory(Model):
    architecture: StrictStr
    packages: list[ImagePackage]
    schema: StrictStr
    subject: StrictStr
    tools: list[ImageTool]
