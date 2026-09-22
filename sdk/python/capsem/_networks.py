"""Typed private-network resources over the authenticated gateway transport."""

from __future__ import annotations

import builtins
from typing import TYPE_CHECKING

from . import _operations as api
from . import models
from ._transport import Transport

if TYPE_CHECKING:
    from .vm import VM


class Networks:
    def __init__(self, transport: Transport) -> None:
        self._transport = transport

    async def create(self, name: str) -> models.NetworkInfo:
        return await api.create_network(
            self._transport, body=models.CreateNetworkRequest(name=name),
        )

    async def list(self) -> builtins.list[models.NetworkInfo]:
        return (await api.list_networks(self._transport)).networks

    async def inspect(self, network_id: str) -> models.NetworkInfo:
        return await api.get_network(self._transport, id=network_id)

    async def delete(self, network: models.NetworkInfo) -> models.VmActionResponse:
        if not isinstance(network, models.NetworkInfo):
            raise TypeError("network must be an object returned by capsem.networks")
        return await api.delete_network(self._transport, id=network.id)

    async def logs(
        self,
        network: models.NetworkInfo,
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
        if not isinstance(network, models.NetworkInfo):
            raise TypeError("network must be an object returned by capsem.networks")
        return await api.get_network_logs(
            self._transport,
            id=network.id,
            cursor=cursor,
            limit=limit,
            vm=vm,
            connection=connection,
            type=event_type,
            decision=decision,
            since=since,
            until=until,
        )


class VmNetworks:
    def __init__(self, vm: VM) -> None:
        self._vm = vm

    async def list(self) -> builtins.list[models.NetworkInfo]:
        vm_id = await self._vm._resolve()
        response = await api.list_networks(self._vm._transport)
        return [network for network in response.networks
                if any(member.vm_id == vm_id for member in network.members)]

    async def attach(self, network: models.NetworkInfo) -> models.NetworkInfo:
        if not isinstance(network, models.NetworkInfo):
            raise TypeError("network must be an object returned by capsem.networks")
        return await api.attach_network_member(
            self._vm._transport, id=network.id, vm_id=await self._vm._resolve(),
        )

    async def detach(self, network: models.NetworkInfo) -> models.NetworkInfo:
        if not isinstance(network, models.NetworkInfo):
            raise TypeError("network must be an object returned by capsem.networks")
        return await api.detach_network_member(
            self._vm._transport, id=network.id, vm_id=await self._vm._resolve(),
        )
