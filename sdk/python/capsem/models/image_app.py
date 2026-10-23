"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr

from .image_evidence import ImageEvidence
from .image_interface import ImageInterface
from .model_base import Model


class ImageApp(Model):
    nonnullable_optional = frozenset(['evidence', 'interfaces'])
    architecture: StrictStr
    catalog_name: StrictStr
    category: StrictStr | None = None
    description: StrictStr
    display_name: StrictStr
    evidence: list[ImageEvidence] | None = None
    icon: StrictStr | None = None
    image: StrictStr
    interfaces: list[ImageInterface] | None = None
    publisher: StrictStr | None = None
    url: StrictStr | None = None
