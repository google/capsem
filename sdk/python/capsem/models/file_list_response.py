"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from .file_list_entry import FileListEntry
from .model_base import Model


class FileListResponse(Model):
    entries: list[FileListEntry]
