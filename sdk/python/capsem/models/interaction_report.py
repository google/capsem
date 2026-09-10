"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from .interaction import Interaction
from .interaction_body import InteractionBody
from .model_base import Model


class InteractionReport(Model):
    bodies: list[InteractionBody]
    items: list[Interaction]
