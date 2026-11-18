"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict

from .model_base import Model, UuidString


class EnvironmentSandboxTarget(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    generation: UuidString
    session_id: UuidString
