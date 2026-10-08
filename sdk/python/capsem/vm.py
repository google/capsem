"""A VM handle resolved through the authenticated HTTP gateway."""

from __future__ import annotations

from collections.abc import Awaitable, Sequence
from pathlib import Path
from typing import TypeVar

from pydantic import StrictStr, TypeAdapter

from . import _operations as api
from . import models
from ._client import Client
from ._container import Container
from ._networks import VmNetworks
from ._ports import Ports
from ._resources import Files, Stats
from ._transport import HttpError, Transport
from .errors import VmNotFoundError, _ErrorContext, _exec_with_timeout, _map_http_error
from .execution import CREATE_READY_SECS, ExecResult, command_deadline

_T = TypeVar("_T")


class VM(Client):
    def __init__(self, url: str | None = None, token: str | None = None, *,
                 name: str | None = None, id: str | None = None,
                 run_dir: str | Path | None = None, timeout: float = 30) -> None:
        super().__init__(url, token, run_dir=run_dir, timeout=timeout)
        self._select(name=name, id=id)

    def _select(self, *, name: str | None, id: str | None) -> None:
        if (name is None) == (id is None):
            raise ValueError("select a VM by exactly one nonempty name or id")
        self._name = TypeAdapter(StrictStr).validate_python(name) if name is not None else None
        self._id = TypeAdapter(StrictStr).validate_python(id) if id is not None else None
        if self._name == "" or self._id == "":
            raise ValueError("select a VM by exactly one nonempty name or id")
        self.files = Files(self)
        self.networks = VmNetworks(self)
        self.stats = Stats(self)
        self.container = Container(self)
        self._has_container: bool | None = None
        self.ports = Ports(self)

    @classmethod
    def _bind(cls, transport: Transport, *, id: str | None = None, name: str | None = None,
              container: bool | None = None) -> VM:
        """Bind a handle to `transport` by `id`, or lazily by `name` when `id` is unknown.

        A handle bound with `id=None` resolves `name` via `GET /vms/list` on its
        first operation; if the name is reused before then, resolution binds to
        whichever sandbox currently holds `name`.
        """
        vm = cls._from_transport(transport)
        vm._select(name=None if id else name, id=id)
        vm._name = name
        vm._has_container = container
        return vm

    @property
    def id(self) -> str | None:
        """Canonical ID, populated by the first request when selecting by name."""
        return self._id

    @property
    def name(self) -> str | None:
        return self._name

    async def _resolve(self) -> str:
        if self._id is None:
            response = await api.list_vms(self._transport)
            matches = [vm for vm in response.sandboxes if vm.name == self._name]
            if not matches:
                raise VmNotFoundError(f"expected one VM named {self._name!r}, found 0", vm_name=self._name)
            if len(matches) != 1:
                raise LookupError(f"expected one VM named {self._name!r}, found {len(matches)}")
            self._id = matches[0].id
        return self._id

    async def _call(self, vm_id: str, op: Awaitable[_T]) -> _T:
        try:
            return await op
        except HttpError as error:
            mapped = _map_http_error(error, _ErrorContext(vm_id=vm_id, vm_name=self._name))
            if mapped is not None:
                raise mapped from error
            raise

    async def _port_target(self) -> models.ExposureTarget:
        if self._has_container is None:
            try:
                await api.get_vm_container(self._transport, id=await self._resolve())
            except HttpError as error:
                if error.status != 404:
                    raise
                self._has_container = False
            else:
                self._has_container = True
        return models.ExposureTarget.CONTAINER if self._has_container else models.ExposureTarget.VM

    async def info(self) -> models.SandboxInfo:
        vm_id = await self._resolve()
        return await self._call(vm_id, api.get_vm_info(self._transport, id=vm_id))

    async def exec(self, command: str, *, timeout_secs: int | None = None,
                   target: models.ExecTarget | None = None) -> ExecResult:
        body = models.ExecRequest.model_validate({
            "command": command, "timeout_secs": timeout_secs,
            **({"target": target} if target is not None else {}),
        })
        vm_id = await self._resolve()
        return await _exec_with_timeout(
            api.exec_vm(
                self._transport, id=vm_id,
                body=body,
                request_timeout=command_deadline(self._transport.timeout, timeout_secs),
            ),
            command=command, timeout_secs=timeout_secs, vm_id=vm_id, vm_name=self._name,
        )

    async def start(self) -> models.ProvisionResponse:
        vm_id = await self._resolve()
        return await self._call(
            vm_id,
            api.start_vm(
                self._transport, id=vm_id,
                request_timeout=command_deadline(self._transport.timeout, CREATE_READY_SECS),
            ),
        )

    async def persist(self, name: str) -> models.PersistResponse:
        vm_id = await self._resolve()
        return await self._call(vm_id, api.persist_vm(
            self._transport, id=vm_id, body=models.PersistRequest(name=name),
        ))

    async def stop(self) -> models.StopResponse:
        vm_id = await self._resolve()
        return await self._call(vm_id, api.stop_vm(self._transport, id=vm_id))

    async def pause(self) -> models.VmActionResponse:
        vm_id = await self._resolve()
        return await self._call(vm_id, api.pause_vm(self._transport, id=vm_id))

    async def resume(self) -> models.ProvisionResponse:
        vm_id = await self._resolve()
        return await self._call(
            vm_id,
            api.resume_vm(
                self._transport, id=vm_id,
                request_timeout=command_deadline(self._transport.timeout, CREATE_READY_SECS),
            ),
        )

    async def delete(self) -> models.VmActionResponse:
        vm_id = await self._resolve()
        return await self._call(vm_id, api.delete_vm(self._transport, id=vm_id))

    async def fork(
        self,
        name: str,
        *,
        description: str | None = None,
        labels: dict[str, str] | None = None,
    ) -> VM:
        """Fork this VM into a new persistent VM.

        Args:
            name: Persistent name for the forked VM.
            description: Optional description for the forked VM.
            labels: Optional labels overriding the source VM's labels (`{}` clears).
        """
        vm_id = await self._resolve()
        response = await self._call(vm_id, api.fork_vm(self._transport, id=vm_id, body=models.ForkRequest(
            name=name, description=description, labels=labels,
        )))
        return VM._bind(
            self._transport, id=response.id, name=response.name, container=self._has_container,
        )

    async def log(self, *, grep: str | None = None, tail: int | None = None,
                  max_bytes: int | None = None) -> models.LogsResponse:
        vm_id = await self._resolve()
        return await self._call(
            vm_id, api.get_vm_logs(self._transport, id=vm_id, grep=grep, tail=tail, max_bytes=max_bytes),
        )

    async def history(self, *, limit: int | None = None, offset: int | None = None,
                      search: str | None = None, layer: models.HistoryLayerFilter | None = None) -> models.HistoryResponse:
        vm_id = await self._resolve()
        return await self._call(
            vm_id, api.get_vm_history(self._transport, id=vm_id, limit=limit, offset=offset, search=search, layer=layer),
        )

    async def timeline(self, *, trace_id: str | None = None, since: str | None = None,
                       limit: int | None = None, layers: Sequence[models.TimelineLayer] | None = None) -> models.TimelineResponse:
        vm_id = await self._resolve()
        return await self._call(
            vm_id,
            api.get_vm_timeline(
                self._transport, id=vm_id, trace_id=trace_id, since=since, limit=limit,
                layers=list(layers) if layers is not None else None,
            ),
        )
