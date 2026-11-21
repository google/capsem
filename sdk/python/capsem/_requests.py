"""Operator inspection of durable requests and their frozen review inputs."""
from __future__ import annotations

from . import _operations as api
from . import models
from ._transport import Transport


class Requests:
    def __init__(self, transport: Transport) -> None:
        self._transport = transport

    async def list(
        self, *, after: int | None = None, limit: int | None = None,
        pending_only: bool | None = None, request_timeout: float | None = None,
    ) -> models.RequestListResponse:
        return await api.list_requests(
            self._transport, after=after, limit=limit, pending_only=pending_only,
            request_timeout=request_timeout,
        )

    async def info(self, id: str, *, request_timeout: float | None = None) -> models.RequestDetailResponse:
        result = await api.get_request_info(self._transport, id=id, request_timeout=request_timeout)
        if result.summary.request_id != id:
            raise ValueError("review request identity changed")
        return result

    async def resolve(
        self, id: str, request: models.RequestResolveRequest, *, request_timeout: float | None = None,
    ) -> models.EnvironmentMutationResponse:
        result = await api.resolve_request(self._transport, id=id, body=request, request_timeout=request_timeout)
        if result.request_id != id:
            raise ValueError("resolved request identity changed")
        return result
