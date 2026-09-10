"""A VM handle resolved through the authenticated HTTP gateway."""

from __future__ import annotations

from collections.abc import Sequence

from pydantic import StrictStr, TypeAdapter

from . import _operations as api
from . import models
from ._client import Client
from ._resources import Copy, Snapshots, Stats
from ._transport import Transport


class VM(Client):
    def __init__(self, url: str, token: str, *, name: str | None = None,
                 id: str | None = None, timeout: float = 30) -> None:
        super().__init__(url, token, timeout=timeout)
        self._select(name=name, id=id)

    def _select(self, *, name: str | None, id: str | None) -> None:
        if bool(name) == bool(id):
            raise ValueError("select a VM by exactly one nonempty name or id")
        self._name = TypeAdapter(StrictStr).validate_python(name) if name else None
        self._id = TypeAdapter(StrictStr).validate_python(id) if id else None
        self.copy = Copy(self)
        self.snapshots = Snapshots(self)
        self.stats = Stats(self)

    @classmethod
    def _bind(cls, transport: Transport, *, id: str, name: str | None = None) -> VM:
        vm = cls._from_transport(transport)
        vm._select(name=None, id=id)
        vm._name = name
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
            if len(matches) != 1:
                raise LookupError(f"expected one VM named {self._name!r}, found {len(matches)}")
            self._id = matches[0].id
        return self._id

    async def info(self) -> models.SandboxInfo:
        return await api.get_vm_info(self._transport, id=await self._resolve())

    async def exec(self, command: str, *, timeout_secs: int | None = None) -> models.ExecResponse:
        return await api.exec_vm(self._transport, id=await self._resolve(), body=models.ExecRequest(
            command=command, timeout_secs=timeout_secs,
        ))

    async def start(self) -> models.ProvisionResponse:
        return await api.start_vm(self._transport, id=await self._resolve())

    async def stop(self) -> models.StopResponse:
        return await api.stop_vm(self._transport, id=await self._resolve())

    async def pause(self) -> models.VmActionResponse:
        return await api.pause_vm(self._transport, id=await self._resolve())

    async def resume(self) -> models.ProvisionResponse:
        return await api.resume_vm(self._transport, id=await self._resolve())

    async def delete(self) -> models.VmActionResponse:
        return await api.delete_vm(self._transport, id=await self._resolve())

    async def fork(self, name: str, *, description: str | None = None) -> VM:
        response = await api.fork_vm(self._transport, id=await self._resolve(), body=models.ForkRequest(
            name=name, description=description,
        ))
        return VM._bind(self._transport, id=response.id, name=response.name)

    async def log(self, *, grep: str | None = None, tail: int | None = None,
                  max_bytes: int | None = None) -> models.LogsResponse:
        return await api.get_vm_logs(self._transport, id=await self._resolve(), grep=grep, tail=tail, max_bytes=max_bytes)

    async def history(self, *, limit: int | None = None, offset: int | None = None,
                      search: str | None = None, layer: models.HistoryLayerFilter | None = None) -> models.HistoryResponse:
        return await api.get_vm_history(self._transport, id=await self._resolve(), limit=limit, offset=offset, search=search, layer=layer)

    async def list(self, path: str = "/", *, depth: int | None = None) -> models.FileListResponse:
        return await api.list_vm_files(self._transport, id=await self._resolve(), path=None if path == "/" else path, depth=depth)

    async def changes(self, checkpoint: str, *, limit: int | None = None,
                      offset: int | None = None) -> models.ChangesResponse:
        return await api.get_vm_changes(self._transport, id=await self._resolve(), checkpoint=checkpoint, limit=limit, offset=offset)

    async def timeline(self, *, trace_id: str | None = None, since: str | None = None,
                       limit: int | None = None, layers: Sequence[models.TimelineLayer] | None = None) -> models.TimelineResponse:
        return await api.get_vm_timeline(self._transport, id=await self._resolve(), trace_id=trace_id, since=since, limit=limit, layers=list(layers) if layers is not None else None)
