"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict

from .interaction_block import InteractionBlock
from .interaction_message_kind import InteractionMessageKind
from .interaction_role import InteractionRole
from .model_base import Model


class InteractionMessage(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    blocks: list[InteractionBlock]
    kind: InteractionMessageKind
    role: InteractionRole
