"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import ConfigDict, Field, StrictInt, StrictStr

from .connection_provider import ConnectionProvider
from .connection_state import ConnectionState
from .credential_error_code import CredentialErrorCode
from .model_base import DateTimeString, Model
from .provider_revocation_state import ProviderRevocationState


class ConnectionSummary(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    account_label: StrictStr | None
    available_capabilities: list[StrictStr]
    created_at: DateTimeString
    error_code: CredentialErrorCode | None
    granted_scopes: list[StrictStr]
    id: StrictStr
    last_refresh_at: DateTimeString | None
    provider: ConnectionProvider
    provider_revocation: ProviderRevocationState
    revision: Annotated[Annotated[StrictInt, Field(ge=0)], Field(le=9007199254740991)]
    scope_profile_id: StrictStr
    state: ConnectionState
    updated_at: DateTimeString
