"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt, StrictStr

from .model_base import Model


class PreviewSessionResponse(Model):
    bootstrap_token: StrictStr
    expires_in_seconds: Annotated[StrictInt, Field(ge=0)]
    url: StrictStr
