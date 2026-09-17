"""Simple port handles over the generated exposure wire contract."""

from __future__ import annotations

import builtins
from dataclasses import dataclass
from typing import Protocol

from . import _operations as api
from . import models
from ._transport import Transport


class _Vm(Protocol):
    @property
    def _transport(self) -> Transport: ...

    async def _resolve(self) -> str: ...

    async def _port_target(self) -> models.ExposureTarget: ...


@dataclass(frozen=True)
class Port:
    id: str
    guest: int
    host: int | None
    authenticate: bool
    url: str | None = None
    bootstrap_token: str | None = None
    expires_in_seconds: int | None = None

    def __repr__(self) -> str:
        # The bootstrap token opens a browser session on the workload; keep it
        # out of logs and tracebacks the way Registry keeps its secrets out.
        token = "<none>" if self.bootstrap_token is None else "<redacted>"
        return (
            f"Port(id={self.id!r}, guest={self.guest!r}, host={self.host!r}, "
            f"authenticate={self.authenticate!r}, url={self.url!r}, "
            f"bootstrap_token={token}, expires_in_seconds={self.expires_in_seconds!r})"
        )


class Ports:
    def __init__(self, vm: _Vm) -> None:
        self._vm = vm

    @staticmethod
    def _port(exposure: models.ExposureInfo) -> Port:
        return Port(
            id=exposure.id,
            guest=exposure.guest_port,
            host=exposure.host_port,
            authenticate=exposure.access is models.ExposureAccess.HTTP_PREVIEW,
        )

    async def open(self, guest: int, *, host: int = 0, authenticate: bool = False) -> Port:
        if isinstance(guest, bool) or not isinstance(guest, int) or not 1 <= guest <= 65_535:
            raise ValueError("guest port must be between 1 and 65535")
        if isinstance(host, bool) or not isinstance(host, int) or not 0 <= host <= 65_535:
            raise ValueError("host port must be between 0 and 65535")
        if authenticate and host:
            raise ValueError("authenticated ports use the gateway origin and cannot select a host port")
        vm_id = await self._vm._resolve()
        exposure = await api.create_vm_exposure(
            self._vm._transport,
            id=vm_id,
            body=models.ExposureRequest(
                guest_port=guest,
                host_port=host,
                target=await self._vm._port_target(),
                access=(models.ExposureAccess.HTTP_PREVIEW
                        if authenticate else models.ExposureAccess.LOOPBACK_TCP),
            ),
        )
        port = self._port(exposure)
        if not authenticate:
            return port
        session = await api.create_vm_preview_session(
            self._vm._transport, id=vm_id, exposure_id=exposure.id,
        )
        return Port(
            id=port.id,
            guest=port.guest,
            host=None,
            authenticate=True,
            url=session.url,
            bootstrap_token=session.bootstrap_token,
            expires_in_seconds=session.expires_in_seconds,
        )

    async def list(self) -> builtins.list[Port]:
        response = await api.list_vm_exposures(self._vm._transport, id=await self._vm._resolve())
        return [self._port(exposure) for exposure in response.exposures]

    async def close(self, port: Port) -> models.VmActionResponse:
        if not isinstance(port, Port):
            raise TypeError("port must be an object returned by vm.ports")
        return await api.delete_vm_exposure(
            self._vm._transport, id=await self._vm._resolve(), exposure_id=port.id,
        )
