"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictStr

from .credential_error_code import CredentialErrorCode
from .model_base import DateTimeString, Model
from .transaction_state import TransactionState


class AuthorizationTransaction(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    connection_id: StrictStr | None
    error_code: CredentialErrorCode | None
    expires_at: DateTimeString
    id: StrictStr
    state: TransactionState
