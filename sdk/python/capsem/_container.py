"""Typed container diagnostics and workload execution."""

from __future__ import annotations

from typing import Protocol

from . import _operations as api
from . import models
from ._transport import Transport
from .execution import ExecResult, command_deadline


class _Vm(Protocol):
    @property
    def _transport(self) -> Transport: ...

    async def _resolve(self) -> str: ...


class Container:
    def __init__(self, vm: _Vm) -> None:
        self._vm = vm

    async def status(self) -> models.ContainerStatusResponse:
        return await api.get_vm_container(self._vm._transport, id=await self._vm._resolve())

    async def exec(
        self,
        command: str,
        *,
        timeout_secs: int | None = None,
        user: str | None = None,
        cwd: str | None = None,
    ) -> ExecResult:
        body = models.ExecRequest.model_validate({
            "command": command,
            "timeout_secs": timeout_secs,
            "target": models.ExecTarget.WORKLOAD,
            **({"user": user} if user is not None else {}),
            **({"cwd": cwd} if cwd is not None else {}),
        })
        response = await api.exec_vm(
            self._vm._transport, id=await self._vm._resolve(),
            body=body,
            request_timeout=command_deadline(self._vm._transport.timeout, timeout_secs),
        )
        return ExecResult.from_wire(response)
