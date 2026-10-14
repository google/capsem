"""Explicit host consent and grants bound to service-owned session generations."""
from __future__ import annotations

from typing import TYPE_CHECKING

from . import _operations as api
from . import models
from ._transport import Transport

if TYPE_CHECKING:
    from .vm import VM


class Connections:
    """Consent never implicitly grants a VM access or launches a browser."""

    def __init__(self, transport: Transport) -> None:
        self._transport = transport

    async def list(
        self, *, limit: int | None = None, cursor: str | None = None, request_timeout: float | None = None,
    ) -> models.ConnectionListResponse:
        return await api.list_connections(self._transport, limit=limit, cursor=cursor, request_timeout=request_timeout)

    async def authorize_google(
        self, request: models.AuthorizeGoogleRequest, *, request_timeout: float | None = None,
    ) -> models.AuthorizeGoogleResponse:
        """The caller retains the request ID, opens the URL and polls explicitly."""
        return await api.authorize_google_connection(
            self._transport, body=request, request_timeout=request_timeout,
        )

    async def authorization(
        self, id: str, *, request_timeout: float | None = None,
    ) -> models.AuthorizationTransaction:
        result = await api.get_connection_authorization(
            self._transport, transaction_id=id, request_timeout=request_timeout,
        )
        if result.id != id:
            raise ValueError("authorization identity changed")
        return result

    async def cancel_authorization(
        self, id: str, *, request_timeout: float | None = None,
    ) -> models.AuthorizationTransaction:
        result = await api.cancel_connection_authorization(
            self._transport, transaction_id=id, request_timeout=request_timeout,
        )
        if result.id != id:
            raise ValueError("authorization identity changed")
        return result

    async def get(self, id: str, *, request_timeout: float | None = None) -> models.ConnectionSummary:
        result = await api.get_connection(
            self._transport, connection_id=id, request_timeout=request_timeout,
        )
        if result.id != id:
            raise ValueError("connection identity changed")
        return result

    async def disconnect(self, id: str, *, request_timeout: float | None = None) -> models.DisconnectResponse:
        """Fence grants; poll explicitly to confirm provider retirement."""
        result = await api.disconnect_connection(
            self._transport, connection_id=id, request_timeout=request_timeout,
        )
        if result.connection.id != id:
            raise ValueError("connection identity changed")
        return result


class CredentialGrants:
    def __init__(self, vm: VM) -> None:
        self._vm = vm

    async def list(self, *, request_timeout: float | None = None) -> models.SessionGrantListResponse:
        id = await self._vm._resolve(request_timeout=request_timeout)
        result = await api.list_session_credential_grants(
            self._vm._transport, id=id, request_timeout=request_timeout,
        )
        if result.session_id != id or any(
            grant.session_id != id or grant.owner_generation != result.owner_generation
            for grant in result.grants
        ):
            raise ValueError("grant session or ownership changed")
        return result

    async def create(
        self, request: models.CreateSessionGrantRequest, *, request_timeout: float | None = None,
    ) -> models.SessionGrantSummary:
        """Explicit approval for this owner; never reconnect or replay a mutation."""
        request = request.model_copy(deep=True)
        id = await self._vm._resolve(request_timeout=request_timeout)
        result = await api.create_session_credential_grant(
            self._vm._transport, id=id, body=request, request_timeout=request_timeout,
        )
        if (result.session_id != id or result.owner_generation != request.expected_owner_generation
                or result.connection_id != request.connection_id
                or set(result.capability_ids) != set(request.capability_ids)
                or len(set(result.capability_ids)) != len(result.capability_ids)):
            raise ValueError("grant session, ownership, connection or capabilities changed")
        return result

    async def revoke(self, grant_id: str, *, request_timeout: float | None = None) -> models.RevokeSessionGrantResponse:
        id = await self._vm._resolve(request_timeout=request_timeout)
        result = await api.revoke_session_credential_grant(
            self._vm._transport, id=id, grant_id=grant_id, request_timeout=request_timeout,
        )
        if result.grant.session_id != id or result.grant.id != grant_id:
            raise ValueError("revoked grant identity changed")
        return result
