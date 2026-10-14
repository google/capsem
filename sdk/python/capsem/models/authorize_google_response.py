"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict, StrictStr

from .authorization_transaction import AuthorizationTransaction
from .model_base import Model


class AuthorizeGoogleResponse(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    authorization_url: StrictStr | None
    transaction: AuthorizationTransaction
