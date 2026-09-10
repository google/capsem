"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictBool

from .model_base import Model


class UpdateApplyRequest(Model):
    nonnullable_optional = frozenset(['confirmed', 'dry_run'])
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    confirmed: StrictBool | None = None
    dry_run: StrictBool | None = None
