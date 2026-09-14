"""Typed private-network resources over the authenticated gateway transport."""

from __future__ import annotations

from . import _operations as api
from . import models
from ._transport import Transport


class Networks:
    def __init__(self, transport: Transport) -> None:
        self._transport = transport

    async def create(self, name: str) -> models.NetworkInfo:
        return await api.create_network(
            self._transport, body=models.CreateNetworkRequest(name=name),
        )

    async def list(self) -> models.NetworkListResponse:
        return await api.list_networks(self._transport)

    async def inspect(self, network_id: str) -> models.NetworkInfo:
        return await api.get_network(self._transport, id=network_id)

    async def delete(self, network_id: str) -> models.VmActionResponse:
        return await api.delete_network(self._transport, id=network_id)

    async def attach(self, network_id: str, vm_id: str) -> models.NetworkInfo:
        return await api.attach_network_member(self._transport, id=network_id, vm_id=vm_id)

    async def detach(self, network_id: str, vm_id: str) -> models.NetworkInfo:
        return await api.detach_network_member(self._transport, id=network_id, vm_id=vm_id)

    async def logs(
        self,
        network_id: str,
        *,
        cursor: str | None = None,
        limit: int | None = None,
        vm: str | None = None,
        connection: str | None = None,
        event_type: str | None = None,
        decision: str | None = None,
        since: int | None = None,
        until: int | None = None,
    ) -> models.NetworkLogsResponse:
        return await api.get_network_logs(
            self._transport,
            id=network_id,
            cursor=cursor,
            limit=limit,
            vm=vm,
            connection=connection,
            type=event_type,
            decision=decision,
            since=since,
            until=until,
        )
