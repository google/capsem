"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr

from .image_evidence_kind import ImageEvidenceKind
from .model_base import Model


class ImageEvidence(Model):
    kind: ImageEvidenceKind
    media_type: StrictStr
    reference: StrictStr
