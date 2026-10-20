"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, Field, StrictStr

from .credential_inject_provider import CredentialInjectProvider
from .credential_storage import CredentialStorage
from .model_base import Model


class CredentialInjectRequest(Model):
    private_input = True
    nonnullable_optional = frozenset(['storage'])
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid", hide_input_in_errors=True)
    provider: CredentialInjectProvider
    storage: CredentialStorage | None = None
    value: StrictStr = Field(repr=False)
