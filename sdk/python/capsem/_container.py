"""Typed read-only container status and wait resource."""

from __future__ import annotations

import asyncio
from typing import Protocol

from . import _operations as api
from . import models
from ._transport import Transport


class _Vm(Protocol):
    @property
    def _transport(self) -> Transport: ...

    async def _resolve(self) -> str: ...


class Container:
    def __init__(self, vm: _Vm) -> None:
        self._vm = vm

    async def status(self) -> models.ContainerStatusResponse:
        return await api.get_vm_container(self._vm._transport, id=await self._vm._resolve())

    async def wait(self, *, interval: float = 0.1) -> models.ContainerStatusResponse:
        if interval <= 0:
            raise ValueError("interval must be positive")
        while True:
            status = await self.status()
            if status.state not in {
                models.ContainerState.PULLING,
                models.ContainerState.STAGING,
                models.ContainerState.STARTING,
            }:
                return status
            await asyncio.sleep(interval)
