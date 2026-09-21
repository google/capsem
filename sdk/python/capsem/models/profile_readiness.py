"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictBool, StrictInt, StrictStr

from .model_base import Model
from .profile_artifact_issue import ProfileArtifactIssue
from .profile_update_semantics import ProfileUpdateSemantics


class ProfileReadiness(Model):
    asset_count: Annotated[StrictInt, Field(ge=0)]
    current_arch: StrictStr
    description: StrictStr
    errors: list[StrictStr]
    id: StrictStr
    invalid_assets: list[ProfileArtifactIssue]
    invalid_files: list[ProfileArtifactIssue]
    missing_assets: list[ProfileArtifactIssue]
    name: StrictStr
    profile_payload_hash: StrictStr | None = None
    ready: StrictBool
    revision: StrictStr | None = None
    update_semantics: ProfileUpdateSemantics | None = None
