"""Hypervisor overview, VM creation, logs and updates through the gateway."""

from __future__ import annotations

import re
from collections.abc import Sequence

from . import _operations as api
from . import models
from ._client import Client
from ._networks import Networks
from ._profiles import Profiles
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
    def __init__(self, url: str, token: str, *, timeout: float = 30) -> None:
        super().__init__(url, token, timeout=timeout)
        self.networks = Networks(self._transport)
        self.profiles = Profiles(self._transport)

    async def info(self) -> models.HypervisorInfo:
        return await api.get_hypervisor_info(self._transport)

    async def list(self) -> models.ListResponse:
        return await api.list_vms(self._transport)

    async def create(self, profile: str, *, name: str = "", vcpu: int | None = None,
                     memory: str | int | None = None, env: dict[str, str] | None = None,
                     networks: Sequence[str] = ()) -> VM:
        if vcpu is not None and vcpu < 1:
            raise ValueError("vcpu must be positive")
        request = models.ProvisionRequest(
            profile_id=profile, name=name or None, persistent=bool(name),
            cpus=vcpu, ram_mb=_memory_mb(memory), env=env, networks=list(networks),
        )
        response = await api.create_vm(self._transport, body=request)
        return VM._bind(self._transport, id=response.id, name=response.name)

    async def log(self, source: models.HostLogSource = models.HostLogSource.SERVICE, *,
                  grep: str | None = None, tail: int | None = None,
                  max_bytes: int | None = None) -> models.HostLogsResponse:
        return await api.get_hypervisor_logs(self._transport, name=source, grep=grep, tail=tail, max_bytes=max_bytes)

    async def run(self, command: str, *, profile: str = "code", timeout_secs: int | None = None,
                  vcpu: int | None = None, memory: str | int | None = None,
                  env: dict[str, str] | None = None) -> models.ExecResponse:
        return await api.run_vm(self._transport, body=models.RunRequest(
            command=command, profile_id=profile, timeout_secs=timeout_secs,
            cpus=vcpu, ram_mb=_memory_mb(memory), env=env,
        ))

    async def purge(self, *, all: bool = False) -> models.PurgeResponse:
        return await api.purge_vms(self._transport, body=models.PurgeRequest(all=all))

    async def panics(self, *, since: str | None = None,
                     limit: int | None = None) -> models.PanicsResponse:
        return await api.get_panics(self._transport, since=since, limit=limit)

    async def triage(self, *, vm_id: str | None = None, since: str | None = None,
                     limit: int | None = None) -> models.TriageResponse:
        return await api.get_triage(self._transport, id=vm_id, since=since, limit=limit)

    async def update(self) -> models.UpdateActionResponse:
        return await api.update_hypervisor(self._transport, body=models.UpdateApplyRequest(confirmed=True))

    async def restart(self) -> models.RestartResponse:
        """Restart an idle managed service; reconnect with a new gateway token."""
        return await api.restart_hypervisor(self._transport)
