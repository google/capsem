"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from enum import StrEnum


class ImageEvidenceKind(StrEnum):
    OBOM = 'obom'
    SBOM = 'sbom'
    ABOM = 'abom'
    PACKAGE_INVENTORY = 'package_inventory'
