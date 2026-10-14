"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import ConfigDict

from .model_base import Model
from .revoked_credential_authority import RevokedCredentialAuthority
from .session_grant_summary import SessionGrantSummary


class RevokeSessionGrantResponse(Model):
    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")
    grant: SessionGrantSummary
    local_authority: RevokedCredentialAuthority
