"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from .captured_payload import CapturedPayload
from .interaction_block_kind import InteractionBlockKind
from .model_base import Model


class InteractionBlock(Model):
    kind: InteractionBlockKind
    payload: CapturedPayload | None = None
