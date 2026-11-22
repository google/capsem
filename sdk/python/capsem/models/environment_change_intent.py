"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import TypeAlias

from .environment_archive_request import EnvironmentArchiveRequest
from .environment_change_request import EnvironmentChangeRequest
from .environment_image_update_request import EnvironmentImageUpdateRequest

EnvironmentChangeIntent: TypeAlias = EnvironmentChangeRequest | EnvironmentArchiveRequest | EnvironmentImageUpdateRequest
