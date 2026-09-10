"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict

from .captured_payload import CapturedPayload
from .interaction_request_kind import InteractionRequestKind
from .model_base import Model


class InteractionRequest(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    kind: InteractionRequestKind
    payload: CapturedPayload | None = None
