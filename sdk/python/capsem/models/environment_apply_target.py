"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict

from .environment_revision_reference import EnvironmentRevisionReference
from .model_base import Model, UuidString


class EnvironmentApplyTarget(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    base: EnvironmentRevisionReference
    generation: UuidString
    session_id: UuidString
