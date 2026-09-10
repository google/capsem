"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictBool, StrictInt, StrictStr

from .file_entry_type import FileEntryType
from .model_base import Model


class FileListEntry(Model):
    children: list[FileListEntry] | None = None
    is_text: StrictBool | None = None
    label: StrictStr | None = None
    mime: StrictStr | None = None
    mtime: Annotated[StrictInt, Field(ge=0)]
    name: StrictStr
    path: StrictStr
    size: Annotated[StrictInt, Field(ge=0)]
    type: FileEntryType
