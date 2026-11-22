"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from .environment_apply_target_preview import EnvironmentApplyTargetPreview
from .model_base import Model


class EnvironmentApplyPreviewResponse(Model):
    targets: list[EnvironmentApplyTargetPreview]
