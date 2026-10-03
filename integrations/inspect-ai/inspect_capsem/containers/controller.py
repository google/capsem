"""Abstract VM controller protocol and command result type for Capsem containers."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Protocol, runtime_checkable

__all__ = [
    "CapsemController",
    "CommandResult",
    "_exec_checked",
]


@dataclass(frozen=True)
class CommandResult:
    """Result of executing a shell command via a Capsem controller."""

    exit_code: int
    stdout: str
    stderr: str
    truncated: bool = False


@runtime_checkable
class CapsemController(Protocol):
    """Protocol for interacting with Capsem VMs and nested containers."""

    async def start_vm(
        self,
        *,
        template: str,
        cpu_count: int,
        ram_gb: int,
        image: str | None = None,
        command: tuple[str, ...] = (),
        env: dict[str, str] | None = None,
    ) -> str: ...

    async def stop_vm(self, vm_id: str) -> None: ...

    async def list_vms(self) -> list[dict[str, Any]]: ...

    async def exec_in_vm(
        self, vm_id: str, command: str, *, timeout: int = 120
    ) -> CommandResult: ...

    async def upload_to_vm(self, vm_id: str, guest_path: str, data: bytes) -> None: ...

    async def download_from_vm(self, vm_id: str, guest_path: str) -> bytes: ...

    async def close(self) -> None: ...


async def _exec_checked(
    controller: CapsemController, vm_id: str, command: str, *, timeout: int, what: str
) -> CommandResult:
    res = await controller.exec_in_vm(vm_id, command, timeout=timeout)
    if res.exit_code != 0:
        msg = f"{what} failed in VM {vm_id} (exit {res.exit_code}): {res.stderr or res.stdout}"
        raise RuntimeError(msg)
    return res
