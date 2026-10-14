"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import ConfigDict, Field, StrictInt

from .connection_summary import ConnectionSummary
from .local_credential_authority import LocalCredentialAuthority
from .model_base import Model


class DisconnectResponse(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    connection: ConnectionSummary
    local_authority: LocalCredentialAuthority
    revoked_grant_count: Annotated[StrictInt, Field(ge=0)]
