"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import TypeAlias

from .environment_archive_request import EnvironmentArchiveRequest
from .environment_change_request import EnvironmentChangeRequest

EnvironmentChangeIntent: TypeAlias = EnvironmentChangeRequest | EnvironmentArchiveRequest
