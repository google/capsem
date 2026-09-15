"""VM subinterfaces for file transfer, snapshot inventory and statistics."""

from __future__ import annotations

from typing import TYPE_CHECKING

from . import _operations as api
from . import models

if TYPE_CHECKING:
    from .vm import VM


class Resource:
    def __init__(self, vm: VM) -> None:
        self._vm = vm


class Copy(Resource):
    async def from_vm(self, path: str) -> bytes:
        return await api.download_vm_file(self._vm._transport, id=await self._vm._resolve(), path=path)

    async def to_vm(self, path: str, data: bytes) -> models.UploadResponse:
        return await api.upload_vm_file(self._vm._transport, id=await self._vm._resolve(), path=path, body=data)


class Snapshots(Resource):
    async def list(self) -> models.SnapshotsList:
        return await api.list_vm_snapshots(self._vm._transport, id=await self._vm._resolve())

    async def status(self) -> models.SnapshotsStatus:
        return await api.get_vm_snapshots_status(self._vm._transport, id=await self._vm._resolve())


class Stats(Resource):
    async def summary(self) -> models.VmStatsSummaryResponse:
        return await api.get_vm_stats_summary(self._vm._transport, id=await self._vm._resolve())

    async def details(self) -> models.VmStatsDetailResponse:
        return await api.get_vm_stats_detail(self._vm._transport, id=await self._vm._resolve())
