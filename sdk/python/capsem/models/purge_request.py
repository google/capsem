"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictBool

from .model_base import Model


class PurgeRequest(Model):
    nonnullable_optional = frozenset(['all'])
    all: StrictBool | None = None
