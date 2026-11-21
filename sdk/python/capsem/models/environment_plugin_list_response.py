"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict

from .environment_plugin_entry import EnvironmentPluginEntry
from .environment_revision_reference import EnvironmentRevisionReference
from .model_base import Model


class EnvironmentPluginListResponse(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    plugins: list[EnvironmentPluginEntry]
    revision: EnvironmentRevisionReference | None
