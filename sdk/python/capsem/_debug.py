"""Agent-oriented diagnostics through authenticated gateway HTTP."""

from __future__ import annotations

from . import _operations as api
from . import models
from ._transport import Transport


class Debug:
    def __init__(self, transport: Transport) -> None:
        self._transport = transport

    async def panics(self, *, since: str | None = None,
                     limit: int | None = None) -> models.PanicsResponse:
        return await api.get_panics(self._transport, since=since, limit=limit)

    async def triage(self, *, vm_id: str | None = None, since: str | None = None,
                     limit: int | None = None) -> models.TriageResponse:
        return await api.get_triage(self._transport, id=vm_id, since=since, limit=limit)
