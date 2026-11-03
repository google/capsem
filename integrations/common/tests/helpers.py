"""Shared fake controllers and test utilities for capsem-integrations tests."""

from __future__ import annotations

import subprocess
from collections.abc import Callable, Mapping, Sequence
from pathlib import Path
from typing import Any

from capsem import models
from capsem_integrations._controller import CommandResult

Result = CommandResult | Callable[[str], CommandResult]
Rules = Sequence[tuple[str, Result]]


def ok(stdout: str = "", stderr: str = "") -> CommandResult:
    return CommandResult(exit_code=0, stdout=stdout, stderr=stderr)


def fail(code: int = 1, stdout: str = "", stderr: str = "boom") -> CommandResult:
    return CommandResult(exit_code=code, stdout=stdout, stderr=stderr)


class FakeManagedScope:
    """Minimal `ManagedSession` test double wrapping a `VM` handle."""

    def __init__(self, vm: Any) -> None:
        self.vm = vm
        self.closed = False

    async def __aenter__(self) -> Any:
        return self.vm

    async def close(self) -> None:
        self.closed = True
        delete_fn = getattr(self.vm, "delete", None)
        if delete_fn is not None:
            await delete_fn()


def exec_response(
    exit_code: int = 0,
    stdout: str = "",
    stderr: str = "",
    *,
    truncated: bool = False,
    stderr_encoding: models.ExecOutputEncoding = models.ExecOutputEncoding.UTF8,
) -> models.ExecResponse:
    return models.ExecResponse(
        exit_code=exit_code,
        stdout=models.ExecOutput(data=stdout, encoding=models.ExecOutputEncoding.UTF8),
        stderr=models.ExecOutput(data=stderr, encoding=stderr_encoding),
        truncated=truncated,
    )


class Scripted:
    """Async controller answering each command from the first rule whose needle it contains."""

    def __init__(self, rules: Rules = ()) -> None:
        self.rules: list[tuple[str, Result]] = list(rules)
        self.commands: list[str] = []
        self.started: list[dict[str, Any]] = []
        self.stopped: list[str] = []
        self.uploads: dict[str, bytes] = {}
        self.files: dict[str, bytes] = {}
        self.download_max_bytes: list[int | None] = []
        self.closed: bool = False

    async def start_vm(self, **kwargs: Any) -> str:
        self.started.append(kwargs)
        return "vm-s"

    async def stop_vm(self, vm_id: str) -> None:
        self.stopped.append(vm_id)

    async def exec_in_vm(self, vm_id: str, command: str, *, timeout: int = 120) -> CommandResult:
        del vm_id, timeout
        self.commands.append(command)
        for needle, res in self.rules:
            if needle in command:
                return res if isinstance(res, CommandResult) else res(command)
        return ok()

    async def upload_to_vm(self, vm_id: str, guest_path: str, data: bytes) -> None:
        del vm_id
        self.uploads[guest_path] = data

    async def download_from_vm(
        self, vm_id: str, guest_path: str, *, max_bytes: int | None = None
    ) -> bytes:
        del vm_id
        self.download_max_bytes.append(max_bytes)
        data = self.files.get(guest_path, b"downloaded")
        return data[: max_bytes + 1] if max_bytes is not None else data

    async def close(self) -> None:
        self.closed = True


class LocalFakeCapsemController:
    """Host-backed controller mapping VM IDs to local directories."""

    def __init__(self, root_dir: Path) -> None:
        self.root_dir = root_dir
        self.vms: dict[str, Path] = {}
        self.started_vms: list[dict[str, Any]] = []
        self.stopped_vms: list[str] = []
        self.commands: list[str] = []
        self.close_count: int = 0

    async def close(self) -> None:
        self.close_count += 1

    async def start_vm(
        self,
        *,
        cpu_count: int,
        ram_gb: int,
        image: str | None = None,
        command: Sequence[str] | None = None,
        env: dict[str, str] | None = None,
        labels: Mapping[str, str] | None = None,
        registry_ca_pem: str | None = None,
    ) -> str:
        del image, command, env, registry_ca_pem
        vm_id = f"vm-fake-{len(self.started_vms)}"
        vm_root = self.root_dir / vm_id
        vm_root.mkdir(parents=True, exist_ok=True)
        self.vms[vm_id] = vm_root
        self.started_vms.append(
            {
                "vm_id": vm_id,
                "cpu_count": cpu_count,
                "ram_gb": ram_gb,
                "labels": dict(labels) if labels else {},
            }
        )
        return vm_id

    async def stop_vm(self, vm_id: str) -> None:
        self.stopped_vms.append(vm_id)
        self.vms.pop(vm_id, None)

    async def upload_to_vm(self, vm_id: str, guest_path: str, data: bytes) -> None:
        del vm_id
        target = Path(guest_path)
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)

    async def download_from_vm(
        self, vm_id: str, guest_path: str, *, max_bytes: int | None = None
    ) -> bytes:
        del vm_id
        target = Path(guest_path)
        with target.open("rb") as f:
            return f.read(max_bytes + 1) if max_bytes is not None else f.read()

    async def exec_in_vm(self, vm_id: str, command: str, *, timeout: int = 120) -> CommandResult:
        del vm_id
        self.commands.append(command)
        proc = subprocess.run(
            ["bash", "-c", command],
            capture_output=True,
            text=True,
            timeout=timeout,
            check=False,
        )
        return CommandResult(
            exit_code=proc.returncode,
            stdout=proc.stdout,
            stderr=proc.stderr,
        )


def _host_timeout_skips() -> set[str]:
    """Self-checks that host-run fakes can pass only with GNU `timeout`."""
    try:
        version = subprocess.run(
            ["timeout", "--version"], capture_output=True, text=True, check=False
        ).stdout
    except OSError:
        return set()
    return {"test_exec_timeout_not_raised_on_fast_signal_death"} if "uutils" in version else set()
