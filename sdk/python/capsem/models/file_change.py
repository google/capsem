"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictBool, StrictInt, StrictStr

from .file_change_kind import FileChangeKind
from .model_base import Model


class FileChange(Model):
    is_symlink: StrictBool
    kind: FileChangeKind
    path: StrictStr
    size: Annotated[StrictInt, Field(ge=0)] | None = None
