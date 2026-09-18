"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt

from .model_base import Model


class PreviewSessionsRevokedResponse(Model):
    revoked: Annotated[StrictInt, Field(ge=0)]
