"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictBool, StrictInt, StrictStr

from .model_base import Model


class UploadResponse(Model):
    container_path: StrictStr | None = None
    size: Annotated[StrictInt, Field(ge=0)]
    success: StrictBool
    vm_path: StrictStr
