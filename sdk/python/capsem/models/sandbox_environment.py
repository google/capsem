"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictBool, StrictStr

from .environment_revision_reference import EnvironmentRevisionReference
from .model_base import Model


class SandboxEnvironment(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    boot: EnvironmentRevisionReference | None = None
    desired: EnvironmentRevisionReference
    live_policy: EnvironmentRevisionReference | None = None
    owner_generation: StrictStr | None = None
    pending_restart: StrictBool | None = None
    policy_digest: StrictStr | None = None
