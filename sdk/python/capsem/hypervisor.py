"""Hypervisor overview, VM creation, logs and updates through the gateway."""

from __future__ import annotations

from collections.abc import Sequence
from pathlib import Path
from typing import Self

from . import _operations as api
from . import models
from ._client import Client
from ._connections import Connections
from ._credentials import Credentials
from ._debug import Debug
from ._environments import Environments
from ._skills import Skills
from ._images import Images
from ._mcp import Mcp
from ._networks import Networks
from ._requests import Requests
from .errors import _create_vm_with_timeout, _exec_with_timeout
from .execution import (
    ExecResult,
    command_deadline,
)
from .managed import ManagedSession
from .readiness import prepare_creation
from .registry import Registry
from .vm import VM

__all__ = ["Hypervisor"]


def _memory_mb(memory: int | None) -> int | None:
    if memory is None:
        return None
    if isinstance(memory, bool) or not isinstance(memory, int) or memory <= 0:
        raise ValueError("memory must be a positive GiB count")
    return memory * 1024


def _provision_request(*, name: str = "", cpus: int | None = None,
                       memory: int | None = None, env: dict[str, str] | None = None,
                       environment: models.EnvironmentRevisionReference | None = None,
                       from_vm: str | None = None,
                       attach: bool = False,
                       labels: dict[str, str] | None = None,
                       networks: Sequence[models.NetworkInfo] = (), image: str | None = None,
                       command: Sequence[str] = (), registry: Registry | None = None) -> models.ProvisionRequest:
    if not isinstance(attach, bool):
        raise TypeError("attach must be a boolean")
    if attach and environment is None:
        raise ValueError("attached execution requires a saved environment")
    if environment is not None:
        if not isinstance(environment, models.EnvironmentRevisionReference):
            raise TypeError("environment must be an EnvironmentRevisionReference")
        environment = models.EnvironmentRevisionReference.model_validate(environment.model_dump())
        if env is not None or image is not None:
            raise ValueError("a saved environment cannot include inline variables or image overrides")
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
        name=name or None, persistent=bool(name), cpus=cpus, ram_mb=_memory_mb(memory),
        env=env if image is None else None, labels=labels, networks=network_names,
    )
    if environment is not None:
        request.environment = environment
    if from_vm is not None:
        request.from_ = from_vm
    if attach:
        request.attach = True
    if wire is not None:
        request.container = wire
    return request


class Hypervisor(Client):
    def __init__(self, url: str | None = None, token: str | None = None, *,
                 run_dir: str | Path | None = None, timeout: float = 30) -> None:
        super().__init__(url, token, run_dir=run_dir, timeout=timeout)
        self.networks = Networks(self._transport)
        self.mcp = Mcp(self._transport)
        self.debug = Debug(self._transport)
        self.images = Images(self._transport)
        self.environments = Environments(self._transport)
        self.skills = Skills(self._transport)
        self.requests = Requests(self._transport)
        self.credentials = Credentials(self._transport)
        self.connections = Connections(self._transport)

    @classmethod
    def connect(cls, url: str | None = None, token: str | None = None, *,
                run_dir: str | Path | None = None, timeout: float = 30) -> Self:
        """Create a `Hypervisor` client using gateway discovery when `url` or `token` is omitted."""
        return cls(url, token, run_dir=run_dir, timeout=timeout)

    async def info(self) -> models.HypervisorInfo:
        return await api.get_hypervisor_info(self._transport)

    async def list(self) -> models.ListResponse:
        return await api.list_vms(self._transport)

    def vm(self, *, name: str | None = None, id: str | None = None) -> VM:
        """A handle for an existing VM on this hypervisor's transport, selected by
        exactly one nonempty name or id."""
        vm = VM._from_transport(self._transport)
        vm._select(name=name, id=id)
        return vm

    async def create(self, *, name: str = "", cpus: int | None = None,
                     memory: int | None = None, env: dict[str, str] | None = None,
                     environment: models.EnvironmentRevisionReference | None = None,
                     from_vm: str | None = None,
                     attach: bool = False,
                     labels: dict[str, str] | None = None,
                     networks: Sequence[models.NetworkInfo] = (), image: str | None = None,
                     command: Sequence[str] = (), registry: Registry | None = None) -> VM:
        """Provision a VM or container workload.

        Args:
            name: Persistent VM name; omit or pass empty string for an ephemeral VM.
            cpus: Optional vCPU count override.
            memory: Optional RAM size in GiB.
            env: Optional environment variables for the VM or container workload.
            environment: Exact approved environment revision; excludes inline workload overrides.
            from_vm: ID or name of a persistent VM whose state to clone.
            attach: Stage the saved environment's image for an attached container stream.
            labels: Optional string labels attached at creation.
            networks: Private networks to attach to the VM at creation.
            image: Optional OCI image reference to run as a container workload.
            command: Optional command argv override for the container workload.
            registry: Optional custom OCI registry configuration for pulling ``image``.
        """
        request = _provision_request(name=name, cpus=cpus, memory=memory, env=env, environment=environment, from_vm=from_vm, attach=attach, labels=labels,
                                     networks=networks, image=image, command=command, registry=registry)
        # Detached readiness includes the exact retained startup deadline.
        request_timeout = await prepare_creation(self._transport, request)
        return await _create_vm_with_timeout(
            api.create_vm(self._transport, body=request, request_timeout=request_timeout),
            self._transport, bind_vm=VM._bind, name=name,
            container=(True if attach or image is not None else None if environment is not None or from_vm is not None else False),
            request_timeout=request_timeout,
        )

    def managed(self, *, cpus: int | None = None, memory: int | None = None,
                env: dict[str, str] | None = None, labels: dict[str, str] | None = None,
                environment: models.EnvironmentRevisionReference | None = None,
                from_vm: str | None = None,
                attach: bool = False,
                networks: Sequence[models.NetworkInfo] = (),
                image: str | None = None, command: Sequence[str] = (), registry: Registry | None = None) -> ManagedSession:
        """An ephemeral ownership scope: renew while entered, await cleanup on exit."""
        return ManagedSession(self._transport, _provision_request(
            cpus=cpus, memory=memory, env=env, environment=environment, from_vm=from_vm, attach=attach, labels=labels, networks=networks, image=image, command=command, registry=registry))

    async def log(self, source: models.HostLogSource = models.HostLogSource.SERVICE, *,
                  grep: str | None = None, tail: int | None = None,
                  max_bytes: int | None = None) -> models.HostLogsResponse:
        return await api.get_hypervisor_logs(self._transport, name=source, grep=grep, tail=tail, max_bytes=max_bytes)

    async def run(self, command: str, *, timeout_secs: int | None = None,
                  cpus: int | None = None, memory: int | None = None,
                  env: dict[str, str] | None = None,
                  environment: models.EnvironmentRevisionReference | None = None) -> ExecResult:
        if environment is not None and env is not None:
            raise ValueError("a saved environment cannot be combined with inline variables")
        body = models.RunRequest(command=command, timeout_secs=timeout_secs,
                                 cpus=cpus, ram_mb=_memory_mb(memory), env=env)
        if environment is not None:
            body.environment = models.EnvironmentRevisionReference.model_validate(environment)
        return await _exec_with_timeout(
            api.run_vm(self._transport, body=body,
                       request_timeout=command_deadline(self._transport.timeout, timeout_secs)),
            command=command, timeout_secs=timeout_secs,
        )

    async def purge(self, *, all: bool = False) -> models.PurgeResponse:
        return await api.purge_vms(self._transport, body=models.PurgeRequest(all=all))

    async def update(self) -> models.UpdateActionResponse:
        return await api.update_hypervisor(self._transport, body=models.UpdateApplyRequest(confirmed=True))

    async def restart(self) -> models.RestartResponse:
        """Restart an idle managed service; reconnect with a new gateway token."""
        return await api.restart_hypervisor(self._transport)
