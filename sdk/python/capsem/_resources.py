"""VM subinterfaces for file transfer and statistics."""

from __future__ import annotations

from typing import TYPE_CHECKING

from . import _operations as api
from . import models

if TYPE_CHECKING:
    from .vm import VM


class Resource:
    def __init__(self, vm: VM) -> None:
        self._vm = vm


class Files(Resource):
    """Paths are the guest's: `/root/x` in a VM, `/workspace/x` in its container,
    or `x` relative to the workspace. `exact=True` takes a path literally,
    relative to the workspace root, even when it is absolute."""

    async def read(self, path: str, *, exact: bool = False) -> bytes:
        vm_id = await self._vm._resolve()
        return await self._vm._call(vm_id, api.download_vm_file(
            self._vm._transport, id=vm_id, path=path, exact=exact or None,
        ))

    async def write(self, path: str, data: bytes, *, exact: bool = False) -> models.UploadResponse:
        vm_id = await self._vm._resolve()
        return await self._vm._call(vm_id, api.upload_vm_file(
            self._vm._transport, id=vm_id, path=path, exact=exact or None, body=data,
        ))

    async def list(self, path: str = "", *, depth: int | None = None, exact: bool = False) -> models.FileListResponse:
        """An empty `path` lists the workspace root."""
        vm_id = await self._vm._resolve()
        return await self._vm._call(vm_id, api.list_vm_files(
            self._vm._transport, id=vm_id,
            path=path or None, depth=depth, exact=exact or None,
        ))


class Stats(Resource):
    async def summary(self) -> models.VmStatsSummaryResponse:
        vm_id = await self._vm._resolve()
        return await self._vm._call(vm_id, api.get_vm_stats_summary(self._vm._transport, id=vm_id))

    async def details(self) -> models.VmStatsDetailResponse:
        vm_id = await self._vm._resolve()
        return await self._vm._call(vm_id, api.get_vm_stats_detail(self._vm._transport, id=vm_id))
