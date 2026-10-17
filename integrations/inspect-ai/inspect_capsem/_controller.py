"""Capsem VM controller (gateway SDK), protocol, and label helpers for Inspect AI."""

from __future__ import annotations

import asyncio
import contextlib
import logging
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from typing import Any, Protocol, runtime_checkable

from capsem import (
    VM,
    ExecTimeoutError,
    HttpError,
    Hypervisor,
    ManagedSession,
    Registry,
    models,
)
from capsem.execution import (
    EXEC_TIMEOUT_CEILING_SECS,
    decode_exec_output,
)

from inspect_capsem._transfer import _OCI_STAGE_DIR, _staged_download, _staged_upload

logger = logging.getLogger(__name__)

_SDK_CALL_TIMEOUT_SECS = 180.0


@dataclass(frozen=True)
class CommandResult:
    """Result of executing a shell command via a Capsem controller."""

    exit_code: int
    stdout: str
    stderr: str
    truncated: bool = False


@runtime_checkable
class CapsemController(Protocol):
    """Protocol for interacting with Capsem VMs."""

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
    ) -> str: ...
    async def stop_vm(self, vm_id: str) -> None: ...
    async def exec_in_vm(
        self, vm_id: str, command: str, *, timeout: int = 120
    ) -> CommandResult: ...
    async def upload_to_vm(self, vm_id: str, guest_path: str, data: bytes) -> None: ...
    async def download_from_vm(
        self, vm_id: str, guest_path: str, *, max_bytes: int | None = None
    ) -> bytes: ...
    async def close(self) -> None: ...


def _normalize_image_ref(image: str | None) -> str | None:
    if not image or not image.strip():
        return None
    ref = image.strip()
    return ref if "://" in ref else f"docker://{ref}"


def truncate_label_value(value: str, max_bytes: int = 255) -> str:
    """Truncate a VM label value to at most `max_bytes` UTF-8 bytes without splitting codepoints."""
    return value.encode("utf-8")[:max_bytes].decode("utf-8", errors="ignore")


class SdkCapsemController:
    """`CapsemController` backed by `capsem.Hypervisor` and `capsem.ManagedSession`."""

    def __init__(
        self,
        hypervisor: Hypervisor | None = None,
        *,
        url: str | None = None,
        token: str | None = None,
        registry_ca_pem: str | None = None,
    ) -> None:
        if hypervisor is None:
            hypervisor = Hypervisor.connect(url, token, timeout=_SDK_CALL_TIMEOUT_SECS)
        self._hypervisor: Hypervisor = hypervisor
        self._registry_ca_pem = registry_ca_pem
        self._sessions: dict[str, VM] = {}
        self._managed_sessions: dict[str, ManagedSession] = {}
        self._oci_vms: set[str] = set()

    async def close(self) -> None:
        for managed in list(self._managed_sessions.values()):
            with contextlib.suppress(Exception):
                await asyncio.wait_for(managed.close(), timeout=30)
        self._managed_sessions.clear()
        with contextlib.suppress(Exception):
            await asyncio.wait_for(self._hypervisor.close(), timeout=30)

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
        norm_image = _normalize_image_ref(image)
        kwargs: dict[str, Any] = {
            "cpus": cpu_count,
            "memory": ram_gb,
        }
        if labels:
            kwargs["labels"] = dict(labels)
        if norm_image:
            kwargs["image"] = norm_image
        if eff_ca := (registry_ca_pem or self._registry_ca_pem):
            kwargs["registry"] = Registry(ca_pem=eff_ca)
        if command is not None:
            kwargs["command"] = list(command)
        if env:
            kwargs["env"] = dict(env)
        managed = self._hypervisor.managed(**kwargs)
        try:
            session = await managed.__aenter__()
        except HttpError as exc:
            if registry_ca_pem and exc.status in (400, 403):
                auth = (norm_image or "").removeprefix("docker://").partition("/")[
                    0
                ] or "127.0.0.1:5055"
                raise RuntimeError(
                    f"capsem-service rejected loopback build registry image {norm_image!r} "
                    f"({exc}). To enable Compose 'build:' / Dockerfile sandboxes, add "
                    f'sources = ["{auth}"] and admit = ["{auth}/inspect-capsem/build"] '
                    "under [images] in settings.toml."
                ) from exc
            raise
        vm_id = str(session.id)
        self._managed_sessions[vm_id] = managed
        self._sessions[vm_id] = session
        if norm_image:
            self._oci_vms.add(vm_id)
        return vm_id

    async def stop_vm(self, vm_id: str) -> None:
        managed = self._managed_sessions.get(vm_id)
        if managed is not None:
            await managed.close()
            self._managed_sessions.pop(vm_id, None)
            self._sessions.pop(vm_id, None)
            self._oci_vms.discard(vm_id)
            return
        self._oci_vms.discard(vm_id)
        session = self._sessions.pop(vm_id, None) or self._hypervisor.vm(id=vm_id)
        try:
            await session.delete()
        except HttpError as exc:
            if exc.status == 404:
                logger.debug("VM %s already gone during stop_vm", vm_id)
            else:
                raise

    def _session_for(self, vm_id: str) -> VM:
        session = self._sessions.get(vm_id)
        if session is None:
            session = self._hypervisor.vm(id=vm_id)
            self._sessions[vm_id] = session
        return session

    async def exec_in_vm(self, vm_id: str, command: str, *, timeout: int = 120) -> CommandResult:
        session = self._session_for(vm_id)
        timeout = min(timeout, EXEC_TIMEOUT_CEILING_SECS)
        target = models.ExecTarget.WORKLOAD if vm_id in self._oci_vms else models.ExecTarget.VM
        try:
            res = await session.exec(command, timeout_secs=timeout, target=target)
        except ExecTimeoutError as exc:
            raise TimeoutError(f"Capsem exec timed out after {timeout}s: {exc}") from exc
        return CommandResult(
            exit_code=int(res.exit_code),
            stdout=decode_exec_output(res.stdout).decode("utf-8", errors="replace"),
            stderr=decode_exec_output(res.stderr).decode("utf-8", errors="replace"),
            truncated=bool(res.truncated),
        )

    async def upload_to_vm(self, vm_id: str, guest_path: str, data: bytes) -> None:
        files = self._session_for(vm_id).files
        stage_dir = _OCI_STAGE_DIR if vm_id in self._oci_vms else None
        await _staged_upload(self, files, vm_id, guest_path, data, stage_dir=stage_dir)

    async def download_from_vm(
        self, vm_id: str, guest_path: str, *, max_bytes: int | None = None
    ) -> bytes:
        files = self._session_for(vm_id).files
        stage_dir = _OCI_STAGE_DIR if vm_id in self._oci_vms else None
        return await _staged_download(
            self, files, vm_id, guest_path, max_bytes=max_bytes, stage_dir=stage_dir
        )
