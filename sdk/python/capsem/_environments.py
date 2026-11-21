"""Canonical blueprints through the authenticated gateway."""
from __future__ import annotations

from . import _operations as api
from . import models
from ._transport import Transport


class Environments:
    def __init__(self, transport: Transport) -> None:
        self._transport = transport

    async def list(
        self, *, search: str | None = None, after: str | None = None,
        limit: int | None = None, request_timeout: float | None = None,
    ) -> models.EnvironmentListResponse:
        """List visible committed environments; follow the cursor to continue search."""
        return await api.list_environments(
            self._transport, search=search, after=after, limit=limit, request_timeout=request_timeout,
        )

    async def create(
        self, request: models.EnvironmentCreateRequest, *, request_timeout: float | None = None,
    ) -> models.EnvironmentMutationResponse:
        """Submit an explicit request ID; approval may precede application."""
        request_id = request.request_id
        result = await api.create_environment(
            self._transport, body=request, request_timeout=request_timeout,
        )
        if result.request_id != request_id:
            raise ValueError("environment request identity changed")
        return result

    async def change(
        self, id: str, request: models.EnvironmentChangeRequest, *, request_timeout: float | None = None,
    ) -> models.EnvironmentMutationResponse:
        """Submit an observed-base change with a caller-chosen request ID."""
        request_id = request.request_id
        result = await api.change_environment(
            self._transport, id=id, body=request, request_timeout=request_timeout,
        )
        if result.request_id != request_id or result.environment_id != id:
            raise ValueError("environment change identity changed")
        return result

    async def info(
        self, id: str, *, request_timeout: float | None = None,
    ) -> models.EnvironmentCurrentResponse:
        """Read the verified current revision and committed provenance."""
        result = await api.get_environment_info(
            self._transport, id=id, request_timeout=request_timeout,
        )
        if result.document.identity.id != id:
            raise ValueError("environment identity changed")
        return result
