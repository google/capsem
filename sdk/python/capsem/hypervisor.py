"""Hypervisor overview, VM creation, logs and updates through the gateway."""

from __future__ import annotations

import re

from . import _operations as api
from . import models
from ._client import Client
from .vm import VM


def _memory_mb(memory: str | int | None) -> int | None:
    if memory is None:
        return None
    if isinstance(memory, str):
        match = re.fullmatch(r"([1-9][0-9]*)(M|G)", memory.upper())
        if match is None:
            raise ValueError("memory must be a positive MB count or size such as '512M' or '8G'")
        return int(match[1]) * (1024 if match[2] == "G" else 1)
    if isinstance(memory, bool) or not isinstance(memory, int) or memory <= 0:
        raise ValueError("memory must be a positive MB count")
    return memory


class Hypervisor(Client):
    async def info(self) -> models.HypervisorInfo:
        return await api.get_hypervisor_info(self._transport)

    async def list(self) -> models.ListResponse:
        return await api.list_vms(self._transport)

    async def create(self, profile: str, *, name: str = "", vcpu: int | None = None,
                     memory: str | int | None = None, env: dict[str, str] | None = None) -> VM:
        if vcpu is not None and vcpu < 1:
            raise ValueError("vcpu must be positive")
        request = models.ProvisionRequest(
            profile_id=profile, name=name or None, persistent=bool(name),
            cpus=vcpu, ram_mb=_memory_mb(memory), env=env,
        )
        response = await api.create_vm(self._transport, body=request)
        return VM._bind(self._transport, id=response.id, name=response.name)

    async def log(self, source: models.HostLogSource = models.HostLogSource.SERVICE, *,
                  grep: str | None = None, tail: int | None = None,
                  max_bytes: int | None = None) -> models.HostLogsResponse:
        return await api.get_hypervisor_logs(self._transport, name=source, grep=grep, tail=tail, max_bytes=max_bytes)

    async def update(self) -> models.UpdateActionResponse:
        return await api.update_hypervisor(self._transport, body=models.UpdateApplyRequest(confirmed=True))
