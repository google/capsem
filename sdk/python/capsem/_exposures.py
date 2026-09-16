"""Typed VM exposure lifecycle through authenticated gateway HTTP."""

from __future__ import annotations

from typing import Protocol

from . import _operations as api
from . import models
from ._transport import Transport


class _Vm(Protocol):
    @property
    def _transport(self) -> Transport: ...

    async def _resolve(self) -> str: ...


class Exposures:
    def __init__(self, vm: _Vm) -> None:
        self._vm = vm

    async def create(self, request: models.ExposureRequest) -> models.ExposureInfo:
        return await api.create_vm_exposure(self._vm._transport, id=await self._vm._resolve(), body=request)

    async def list(self) -> models.ExposureListResponse:
        return await api.list_vm_exposures(self._vm._transport, id=await self._vm._resolve())

    async def delete(self, exposure_id: str) -> models.VmActionResponse:
        return await api.delete_vm_exposure(
            self._vm._transport,
            id=await self._vm._resolve(),
            exposure_id=exposure_id,
        )

    async def preview_session(self, exposure_id: str) -> models.PreviewSessionResponse:
        return await api.create_vm_preview_session(
            self._vm._transport,
            id=await self._vm._resolve(),
            exposure_id=exposure_id,
        )
