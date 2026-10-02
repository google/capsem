"""Capsem VM controller (gateway SDK) for Inspect AI."""

from __future__ import annotations

import asyncio
import contextlib
import logging
import os
import posixpath
import re
import shlex
import uuid
from collections.abc import Awaitable, Callable
from pathlib import Path
from typing import Any

from capsem import HttpError, Hypervisor
from capsem.execution import (
    CREATE_READY_SECS,
    EXEC_TIMEOUT_CEILING_SECS,
    GATEWAY_REQUEST_BUDGET_SECS,
    decode_exec_output,
)

from inspect_capsem.containers.controller import (
    CapsemController,
    CommandResult,
    _exec_checked,
)

__all__ = [
    "CapsemController",
    "CommandResult",
    "SdkCapsemController",
]

logger = logging.getLogger(__name__)

_MANAGED_VM_PREFIX = "inspect-capsem-"
# capsem_api::MAX_REQUEST_BODY_BYTES is 10 MiB for request and response bodies; parts stay under it.
_XFER_PART_BYTES = 8 * 1024 * 1024
# The gateway Files API reaches only the VM's /root; transfers stage there.
_XFER_STAGE_DIR = "/root"
# Deadline for one non-exec SDK call (delete, list, file part).
_SDK_CALL_TIMEOUT_SECS = 180.0
_BUILTIN_TEMPLATES = ("default", "code")
# Matches `sanitize_file_path` in `crates/capsem-service/src/fs_utils.rs`.
_SAFE_STAGE_REL_RE = re.compile(r"^[A-Za-z0-9._\-/]+$")


def _rel_to_stage_dir(guest_path: str, stage_dir: str) -> str | None:
    norm_stage = posixpath.normpath(stage_dir)
    norm_guest = posixpath.normpath(guest_path)
    prefix = "/" if norm_stage == "/" else f"{norm_stage}/"
    if not norm_guest.startswith(prefix):
        return None
    rel = norm_guest[len(prefix) :]
    if not rel or ".." in rel or _SAFE_STAGE_REL_RE.fullmatch(rel) is None:
        return None
    return rel


async def _staged_upload(
    controller: CapsemController,
    vm_id: str,
    guest_path: str,
    data: bytes,
    write_part: Callable[[str, bytes], Awaitable[None]],
) -> None:
    dest_q = shlex.quote(guest_path)
    parent_q = shlex.quote(posixpath.dirname(guest_path) or "/")
    if not data:
        await _exec_checked(
            controller,
            vm_id,
            f"mkdir -p {parent_q} && : > {dest_q}",
            timeout=60,
            what="Empty upload",
        )
        return
    stage_dir = _XFER_STAGE_DIR
    part_bytes = _XFER_PART_BYTES
    rel = _rel_to_stage_dir(guest_path, stage_dir)
    if rel is not None and len(data) <= part_bytes:
        try:
            await write_part(rel, data)
            return
        except HttpError as exc:
            if exc.status not in (403, 413):
                raise
    stage = f".capsem-xfer-{uuid.uuid4().hex[:12]}"
    stage_q = shlex.quote(posixpath.join(stage_dir, stage))
    cleaned_inline = False
    try:
        for index, offset in enumerate(range(0, len(data), part_bytes)):
            part = data[offset : offset + part_bytes]
            await write_part(f"{stage}/{index:06d}", part)
        res = await controller.exec_in_vm(
            vm_id,
            f"mkdir -p {parent_q} && cat {stage_q}/* > {dest_q}; "
            f"__ec=$?; rm -rf {stage_q}; exit $__ec",
            timeout=300,
        )
        cleaned_inline = True
        if res.exit_code != 0:
            msg = (
                f"Assembling upload to {guest_path} failed in VM {vm_id} "
                f"(exit {res.exit_code}): {res.stderr or res.stdout}"
            )
            raise RuntimeError(msg)
    finally:
        if not cleaned_inline:
            with contextlib.suppress(Exception):
                await controller.exec_in_vm(vm_id, f"rm -rf {stage_q}", timeout=60)


async def _staged_download(
    controller: CapsemController,
    vm_id: str,
    guest_path: str,
    read_part: Callable[[str], Awaitable[bytes]],
) -> bytes:
    stage_dir = _XFER_STAGE_DIR
    part_bytes = _XFER_PART_BYTES
    rel = _rel_to_stage_dir(guest_path, stage_dir)
    if rel is not None:
        try:
            return await read_part(rel)
        except HttpError as exc:
            if exc.status not in (403, 413):
                raise
    stage = f".capsem-xfer-{uuid.uuid4().hex[:12]}"
    stage_q = shlex.quote(posixpath.join(stage_dir, stage))
    try:
        listing = await _exec_checked(
            controller,
            vm_id,
            f"mkdir -p {stage_q} && cd {stage_q} && "
            f"split -b {part_bytes} -d -a 6 {shlex.quote(guest_path)} part. && "
            f"ls -1",
            timeout=300,
            what=f"Splitting {guest_path} for download",
        )
        names = sorted(line.strip() for line in listing.stdout.splitlines() if line.strip())
        parts: list[bytes] = []
        for name in names:
            parts.append(await read_part(f"{stage}/{name}"))
        return b"".join(parts)
    finally:
        with contextlib.suppress(Exception):
            await controller.exec_in_vm(vm_id, f"rm -rf {stage_q}", timeout=60)


def _capsem_run_dir() -> Path:
    """Resolve the Capsem run directory matching `capsem_foundation::paths::capsem_run_dir`."""
    run_dir = os.environ.get("CAPSEM_RUN_DIR", "").strip()
    if run_dir:
        return Path(run_dir).expanduser()
    capsem_home = os.environ.get("CAPSEM_HOME", "").strip()
    if capsem_home:
        return Path(capsem_home).expanduser() / "run"
    return Path.home() / ".capsem" / "run"


def _resolve_default_gateway_url(url: str | None) -> str:
    if url:
        return url
    env_url = os.environ.get("CAPSEM_GATEWAY_URL")
    if env_url:
        return env_url
    port_file = _capsem_run_dir() / "gateway.port"
    if port_file.is_file():
        port = port_file.read_text(encoding="utf-8").strip()
        if port:
            return f"http://127.0.0.1:{port}"
    return "http://127.0.0.1:19222"


def _resolve_default_gateway_token(token: str | None) -> str:
    if token is not None:
        return token
    env_token = os.environ.get("CAPSEM_GATEWAY_TOKEN")
    if env_token is not None:
        return env_token
    token_file = _capsem_run_dir() / "gateway.token"
    if token_file.is_file():
        return token_file.read_text(encoding="utf-8").strip()
    return ""


class SdkCapsemController:
    """`CapsemController` backed by `capsem.Hypervisor` and `capsem.VM`."""

    def __init__(
        self, hypervisor: Any | None = None, *, url: str | None = None, token: str | None = None
    ) -> None:
        self._call_timeout_secs = _SDK_CALL_TIMEOUT_SECS
        if hypervisor is None:
            gw_url = _resolve_default_gateway_url(url)
            gw_token = _resolve_default_gateway_token(token)
            hypervisor = Hypervisor(gw_url, gw_token, timeout=self._call_timeout_secs)
        self._hypervisor = hypervisor
        self._sessions: dict[str, Any] = {}

    async def close(self) -> None:
        """Close the underlying Hypervisor transport."""
        try:
            await asyncio.wait_for(self._hypervisor.close(), timeout=30)
        except Exception:
            logger.debug("Ignoring error closing hypervisor", exc_info=True)

    async def _resolve_profile_for_template(self, template: str) -> Any | None:
        try:
            profiles = await asyncio.wait_for(
                self._hypervisor.profiles.list(), timeout=self._call_timeout_secs
            )
        except TimeoutError as exc:
            msg = f"Capsem SDK call did not finish within {self._call_timeout_secs:.0f}s"
            raise TimeoutError(msg) from exc
        for prof in profiles or ():
            if template in (prof.id, prof.name):
                return prof
        return None

    async def _create_session_for_template(
        self,
        *,
        vm_name: str,
        template: str,
        cpu_count: int,
        ram_gb: int,
        image: str | None = None,
        command: tuple[str, ...] = (),
        env: dict[str, str] | None = None,
    ) -> Any:
        is_builtin_template = template in ("", *_BUILTIN_TEMPLATES)
        matched_profile: Any | None = None
        if not is_builtin_template:
            matched_profile = await self._resolve_profile_for_template(template)
            if matched_profile is None:
                msg = f"Unsupported or unknown Capsem template/profile: {template!r}"
                raise NotImplementedError(msg)

        # Passing `name` sets `persistent=True` on the gateway so `sweep_leftover_vms` and
        # `cli_cleanup` can locate VMs created here by their `inspect-capsem-` prefix.
        kwargs: dict[str, Any] = {"name": vm_name, "cpus": cpu_count, "memory": ram_gb}
        if matched_profile is not None:
            kwargs["profile"] = matched_profile
        if image:
            kwargs["image"] = image if "://" in image else f"docker://{image}"
        if command:
            kwargs["command"] = list(command)
        if env:
            kwargs["env"] = dict(env)
        # `Hypervisor.create` passes `request_timeout = max(transport.timeout,
        # CREATE_READY_SECS + GATEWAY_REQUEST_BUDGET_SECS)` (230s); `wait_for` is an outer safety bound.
        wait = float(CREATE_READY_SECS + GATEWAY_REQUEST_BUDGET_SECS)
        try:
            return await asyncio.wait_for(self._hypervisor.create(**kwargs), timeout=wait)
        except TimeoutError as exc:
            msg = f"Capsem SDK call did not finish within {wait:.0f}s"
            raise TimeoutError(msg) from exc
        except BaseException as exc:
            await self._cleanup_failed_create(exc)
            raise

    async def _cleanup_failed_create(self, exc: BaseException) -> None:
        if not (isinstance(exc, HttpError) and exc.status == 504):
            return
        match = re.search(r"\bVM\s+([A-Za-z0-9_-]+)\b", exc.body)
        if not match:
            return
        with contextlib.suppress(Exception):
            await self.stop_vm(match.group(1))

    async def start_vm(
        self,
        *,
        template: str,
        cpu_count: int,
        ram_gb: int,
        image: str | None = None,
        command: tuple[str, ...] = (),
        env: dict[str, str] | None = None,
    ) -> str:
        # Managed prefix lets `sweep_leftover_vms` and `cli_cleanup` find VMs created here.
        vm_name = f"{_MANAGED_VM_PREFIX}{uuid.uuid4().hex[:8]}"
        session = await self._create_session_for_template(
            vm_name=vm_name,
            template=template,
            cpu_count=cpu_count,
            ram_gb=ram_gb,
            image=image,
            command=command,
            env=env,
        )
        vm_id = str(session.id)
        self._sessions[vm_id] = session
        return vm_id

    async def stop_vm(self, vm_id: str, *, timeout: float = _SDK_CALL_TIMEOUT_SECS) -> None:
        session = self._sessions.pop(vm_id, None)
        if session is None:
            session = self._hypervisor.vm(id=vm_id)
        try:
            await asyncio.wait_for(session.delete(), timeout=timeout)
        except TimeoutError as exc:
            msg = f"Capsem SDK call did not finish within {timeout:.0f}s"
            raise TimeoutError(msg) from exc
        except HttpError as exc:
            if exc.status == 404:
                logger.debug("VM %s already gone during stop_vm", vm_id)
            else:
                raise

    async def list_vms(self, *, timeout: float = _SDK_CALL_TIMEOUT_SECS) -> list[dict[str, Any]]:
        try:
            remote = await asyncio.wait_for(self._hypervisor.list(), timeout=timeout)
        except TimeoutError as exc:
            msg = f"Capsem SDK call did not finish within {timeout:.0f}s"
            raise TimeoutError(msg) from exc
        return [
            {
                "id": str(item.id or item.name or ""),
                "name": str(item.name or item.id or ""),
                "status": str(item.status.value),
                "persistent": bool(item.persistent),
            }
            for item in remote.sandboxes
        ]

    def _session_for(self, vm_id: str) -> Any:
        session = self._sessions.get(vm_id)
        if session is None:
            session = self._hypervisor.vm(id=vm_id)
            self._sessions[vm_id] = session
        return session

    async def exec_in_vm(self, vm_id: str, command: str, *, timeout: int = 120) -> CommandResult:
        session = self._session_for(vm_id)
        timeout = min(timeout, EXEC_TIMEOUT_CEILING_SECS)
        wait = float(timeout + GATEWAY_REQUEST_BUDGET_SECS)
        try:
            res = await asyncio.wait_for(session.exec(command, timeout_secs=timeout), timeout=wait)
        except TimeoutError as exc:
            msg = f"Capsem SDK call did not finish within {wait:.0f}s"
            raise TimeoutError(msg) from exc
        except HttpError as exc:
            if exc.status in (408, 504) or (
                exc.status == 500 and "IPC command timed out" in exc.body
            ):
                msg = f"Capsem exec timed out after {timeout}s: {exc}"
                raise TimeoutError(msg) from exc
            raise
        return CommandResult(
            exit_code=int(res.exit_code),
            stdout=decode_exec_output(res.stdout).decode("utf-8", errors="replace"),
            stderr=decode_exec_output(res.stderr).decode("utf-8", errors="replace"),
            truncated=bool(res.truncated),
        )

    async def upload_to_vm(self, vm_id: str, guest_path: str, data: bytes) -> None:
        """Write `data` to `guest_path` in the VM, directly under /root or via staged parts."""
        files = self._session_for(vm_id).files

        async def _write_part(rel_dest: str, part: bytes) -> None:
            await asyncio.wait_for(files.write(rel_dest, part), timeout=self._call_timeout_secs)

        await _staged_upload(self, vm_id, guest_path, data, _write_part)

    async def download_from_vm(self, vm_id: str, guest_path: str) -> bytes:
        """Read `guest_path` from the VM, directly under /root or split into staged parts."""
        files = self._session_for(vm_id).files

        async def _read_part(rel_src: str) -> bytes:
            return bytes(
                await asyncio.wait_for(files.read(rel_src), timeout=self._call_timeout_secs)
            )

        return await _staged_download(self, vm_id, guest_path, _read_part)
