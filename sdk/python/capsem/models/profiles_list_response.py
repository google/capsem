"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from .model_base import Model
from .profile_summary import ProfileSummary


class ProfilesListResponse(Model):
    profiles: list[ProfileSummary]
