"""Capsem VM controller (gateway SDK), protocol, and helpers for Inspect AI."""

from __future__ import annotations

import asyncio
import contextlib
import logging
import os
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from typing import Any, Protocol, runtime_checkable

from capsem import (
    VM,
    CreateTimeoutError,
    ExecTimeoutError,
    HttpError,
    Hypervisor,
    Registry,
    models,
)
from capsem.execution import (
    EXEC_TIMEOUT_CEILING_SECS,
    decode_exec_output,
)

from inspect_capsem._transfer import _OCI_STAGE_DIR, _staged_download, _staged_upload

logger = logging.getLogger(__name__)

_MANAGED_BY_LABEL, _MANAGED_BY_VALUE = "managed-by", "inspect-capsem"
_PREFIX_LABEL, _TASK_LABEL = "inspect-capsem-prefix", "inspect-capsem-task"
_NONCE_LABEL = "inspect-capsem-nonce"
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
    async def list_vms(self) -> list[models.SandboxInfo]: ...
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


def _managed_vm_prefix_slug() -> str:
    raw = os.environ.get("CAPSEM_VM_PREFIX", "").strip()
    if not raw:
        return ""
    chars = [
        ch if ch.isalnum() and ch.isascii() else "-" for ch in raw.removeprefix("inspect-capsem-")
    ]
    return "-".join(part for part in "".join(chars).split("-") if part)[:63]


def _managed_vm_labels(
    extra: Mapping[str, str] | None = None,
    *,
    task_name: str | None = None,
    nonce: str | None = None,
) -> dict[str, str]:
    labels = {_MANAGED_BY_LABEL: _MANAGED_BY_VALUE}
    if prefix := _managed_vm_prefix_slug():
        labels[_PREFIX_LABEL] = prefix
    if task_name:
        labels[_TASK_LABEL] = task_name[:255]
    if nonce:
        labels[_NONCE_LABEL] = nonce
    if extra:
        labels.update(extra)
    return labels


def _is_managed_vm(vm_info: models.SandboxInfo, prefix_slug: str | None = None) -> bool:
    if vm_info.persistent:
        return False
    labels = vm_info.labels or {}
    if labels.get(_MANAGED_BY_LABEL) != _MANAGED_BY_VALUE:
        return False
    return str(labels.get(_PREFIX_LABEL) or "") == (
        _managed_vm_prefix_slug() if prefix_slug is None else prefix_slug
    )


class SdkCapsemController:
    """`CapsemController` backed by `capsem.Hypervisor` and `capsem.VM`."""

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
        self._oci_vms: set[str] = set()

    async def close(self) -> None:
        with contextlib.suppress(Exception):
            await asyncio.wait_for(self._hypervisor.close(), timeout=30)

    async def _cleanup_failed_create(self, exc: BaseException) -> None:
        if isinstance(exc, CreateTimeoutError) and exc.vm_id:
            with contextlib.suppress(Exception):
                await self.stop_vm(exc.vm_id)

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
            "labels": _managed_vm_labels(labels),
        }
        if norm_image:
            kwargs["image"] = norm_image
        if eff_ca := (registry_ca_pem or self._registry_ca_pem):
            kwargs["registry"] = Registry(ca_pem=eff_ca)
        if command is not None:
            kwargs["command"] = list(command)
        if env:
            kwargs["env"] = dict(env)
        try:
            session = await self._hypervisor.create(**kwargs)
        except CreateTimeoutError as exc:
            await self._cleanup_failed_create(exc)
            raise
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
        self._sessions[vm_id] = session
        if norm_image:
            self._oci_vms.add(vm_id)
        return vm_id

    async def stop_vm(self, vm_id: str) -> None:
        session = self._sessions.pop(vm_id, None) or self._hypervisor.vm(id=vm_id)
        try:
            await session.delete()
        except HttpError as exc:
            if exc.status == 404:
                logger.debug("VM %s already gone during stop_vm", vm_id)
            else:
                raise

    async def list_vms(self) -> list[models.SandboxInfo]:
        return list((await self._hypervisor.list()).sandboxes)

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
