"""Hypervisor overview, VM creation, logs and updates through the gateway."""

from __future__ import annotations

from collections.abc import Sequence

from . import _operations as api
from . import models
from ._client import Client
from ._debug import Debug
from ._networks import Networks
from ._profiles import Profiles
from .execution import ExecResult
from .registry import Registry
from .vm import VM


def _memory_mb(memory: int | None) -> int | None:
    if memory is None:
        return None
    if isinstance(memory, bool) or not isinstance(memory, int) or memory <= 0:
        raise ValueError("memory must be a positive GiB count")
    return memory * 1024


def _profile_id(profile: models.ProfileSummary | None) -> str:
    if profile is None:
        return "code"
    if not isinstance(profile, models.ProfileSummary):
        raise TypeError("profile must be an object returned by capsem.profiles.list()")
    return profile.id


class Hypervisor(Client):
    def __init__(self, url: str, token: str, *, timeout: float = 30) -> None:
        super().__init__(url, token, timeout=timeout)
        self.networks = Networks(self._transport)
        self.profiles = Profiles(self._transport)
        self.debug = Debug(self._transport)

    async def info(self) -> models.HypervisorInfo:
        return await api.get_hypervisor_info(self._transport)

    async def list(self) -> models.ListResponse:
        return await api.list_vms(self._transport)

    async def create(self, *, profile: models.ProfileSummary | None = None,
                     name: str = "", cpus: int | None = None,
                     memory: int | None = None, env: dict[str, str] | None = None,
                     networks: Sequence[models.NetworkInfo] = (), image: str | None = None,
                     command: Sequence[str] = (), registry: Registry | None = None) -> VM:
        if cpus is not None and (isinstance(cpus, bool) or not isinstance(cpus, int) or cpus < 1):
            raise ValueError("cpus must be positive")
        if image is None and (command or registry is not None):
            raise ValueError("container command and registry require an image")
        if registry is not None and not isinstance(registry, Registry):
            raise TypeError("registry must be a Registry object")
        if image is not None and (not isinstance(image, str) or not image):
            raise ValueError("image must be a nonempty string")
        network_names: list[str] = []
        for network in networks:
            if not isinstance(network, models.NetworkInfo):
                raise TypeError("networks must contain NetworkInfo objects returned by capsem.networks")
            network_names.append(network.name)
        wire: models.ContainerSpec | None = None
        if image is not None:
            wire = models.ContainerSpec(
                image=image, args=list(command), env=env or {}, attach=False,
            ) if registry is None else models.ContainerSpec(
                image=image, args=list(command), env=env or {}, registry=registry._wire(), attach=False,
            )
        request = models.ProvisionRequest(
            profile_id=_profile_id(profile),
            name=name or None, persistent=bool(name),
            cpus=cpus, ram_mb=_memory_mb(memory),
            env=env if image is None else None, networks=network_names,
        )
        if wire is not None:
            request.container = wire
        response = await api.create_vm(self._transport, body=request)
        return VM._bind(self._transport, id=response.id, name=response.name, container=image is not None)

    async def log(self, source: models.HostLogSource = models.HostLogSource.SERVICE, *,
                  grep: str | None = None, tail: int | None = None,
                  max_bytes: int | None = None) -> models.HostLogsResponse:
        return await api.get_hypervisor_logs(self._transport, name=source, grep=grep, tail=tail, max_bytes=max_bytes)

    async def run(self, command: str, *, profile: models.ProfileSummary | None = None,
                  timeout_secs: int | None = None,
                  cpus: int | None = None, memory: int | None = None,
                  env: dict[str, str] | None = None) -> ExecResult:
        response = await api.run_vm(self._transport, body=models.RunRequest(
            command=command, profile_id=_profile_id(profile),
            timeout_secs=timeout_secs,
            cpus=cpus, ram_mb=_memory_mb(memory), env=env,
        ))
        return ExecResult.from_wire(response)

    async def purge(self, *, all: bool = False) -> models.PurgeResponse:
        return await api.purge_vms(self._transport, body=models.PurgeRequest(all=all))

    async def update(self) -> models.UpdateActionResponse:
        return await api.update_hypervisor(self._transport, body=models.UpdateApplyRequest(confirmed=True))

    async def restart(self) -> models.RestartResponse:
        """Restart an idle managed service; reconnect with a new gateway token."""
        return await api.restart_hypervisor(self._transport)
