"""Container runtime boundary types (`ContainerCommandResult`, `ContainerController`)."""

from __future__ import annotations

from typing import Protocol


class ContainerCommandResult(Protocol):
    """Structural protocol for command execution results inside a Capsem VM or container."""

    @property
    def exit_code(self) -> int: ...
    @property
    def stdout(self) -> str: ...
    @property
    def stderr(self) -> str: ...
    @property
    def truncated(self) -> bool: ...


class ContainerController(Protocol):
    """Protocol abstracting Capsem host/gateway operations for container setup."""

    async def exec_in_vm(
        self, vm_id: str, command: str, *, timeout: int = 120
    ) -> ContainerCommandResult: ...
    async def upload_to_vm(self, vm_id: str, guest_path: str, data: bytes) -> None: ...
