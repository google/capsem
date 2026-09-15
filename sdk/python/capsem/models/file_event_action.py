"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from enum import StrEnum


class FileEventAction(StrEnum):
    CREATED = 'created'
    MODIFIED = 'modified'
    DELETED = 'deleted'
    RESTORED = 'restored'
    READ = 'read'
    IMPORT = 'import'
    EXPORT = 'export'
