"""Typed read-only container diagnostics."""

from __future__ import annotations

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
