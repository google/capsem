"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from enum import StrEnum


class FileChangeKind(StrEnum):
    CREATED = 'created'
    MODIFIED = 'modified'
    DELETED = 'deleted'
