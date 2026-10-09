"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictStr

from .credential_storage import CredentialStorage
from .model_base import Model


class CredentialInjectResponse(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    credential_ref: StrictStr
    storage: CredentialStorage
