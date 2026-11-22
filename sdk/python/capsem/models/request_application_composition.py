"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictBool, StrictStr

from .environment_revision_reference import EnvironmentRevisionReference
from .model_base import Model


class RequestApplicationComposition(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    boot: EnvironmentRevisionReference
    corporate_policy_id: StrictStr
    desired: EnvironmentRevisionReference
    live_policy: EnvironmentRevisionReference
    pending_restart: StrictBool
