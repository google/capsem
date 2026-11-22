"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr

from .environment_apply_target import EnvironmentApplyTarget
from .environment_revision_reference import EnvironmentRevisionReference
from .model_base import Model


class EnvironmentApplyTargetPreview(Model):
    boot: EnvironmentRevisionReference
    live_changes: list[StrictStr]
    live_policy: EnvironmentRevisionReference
    next_start_changes: list[StrictStr]
    target: EnvironmentApplyTarget
