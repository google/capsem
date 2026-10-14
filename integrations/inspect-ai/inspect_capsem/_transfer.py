"""Staged multi-part file upload and download helpers for Capsem VMs."""

from __future__ import annotations

import contextlib
import posixpath
import re
import shlex
import uuid
from typing import TYPE_CHECKING, Protocol

from capsem import HttpError
from capsem.execution import MAX_REQUEST_BODY_BYTES

if TYPE_CHECKING:
    from inspect_capsem._controller import CapsemController, CommandResult


class _FilesClient(Protocol):
    async def read(self, path: str, /) -> bytes: ...
    async def write(self, path: str, data: bytes, /) -> object: ...


_XFER_PART_BYTES = MAX_REQUEST_BODY_BYTES - 2 * 1024 * 1024
_XFER_STAGE_DIR = "/root"
# Direct Files API eligibility check: the service's path filter silently strips
# characters outside `[A-Za-z0-9._-/]` instead of returning 400, so only
# relative paths that the service stores verbatim take the direct Files API fast
# path; all other filenames route through staged `exec` (`shlex.quote`).
_DIRECT_REL_RE = re.compile(r"[A-Za-z0-9._-]+(?:/[A-Za-z0-9._-]+)*\Z")


async def _exec_checked(
    controller: CapsemController, vm_id: str, command: str, *, timeout: int, what: str
) -> CommandResult:
    res = await controller.exec_in_vm(vm_id, command, timeout=timeout)
    if res.exit_code != 0:
        raise RuntimeError(
            f"{what} failed in VM {vm_id} (exit {res.exit_code}): {res.stderr or res.stdout}"
        )
    return res


def _rel_to_stage_dir(guest_path: str, stage_dir: str) -> str | None:
    norm_stage = posixpath.normpath(stage_dir)
    norm_guest = posixpath.normpath(guest_path)
    prefix = "/" if norm_stage == "/" else f"{norm_stage}/"
    if not norm_guest.startswith(prefix):
        return None
    rel = norm_guest[len(prefix) :]
    if not rel or not _DIRECT_REL_RE.match(rel) or ".." in rel.split("/"):
        return None
    return rel


async def _staged_upload(
    controller: CapsemController,
    files: _FilesClient,
    vm_id: str,
    guest_path: str,
    data: bytes,
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
            await files.write(rel, data)
            return
        except HttpError as exc:
            if exc.status not in (400, 403, 413):
                raise
    stage = f".capsem-xfer-{uuid.uuid4().hex[:12]}"
    stage_q = shlex.quote(posixpath.join(stage_dir, stage))
    cleaned_inline = False
    try:
        for index, offset in enumerate(range(0, len(data), part_bytes)):
            part = data[offset : offset + part_bytes]
            await files.write(f"{stage}/{index:06d}", part)
        res = await controller.exec_in_vm(
            vm_id,
            f"mkdir -p {parent_q} && cat {stage_q}/* > {dest_q}; "
            f"__ec=$?; rm -rf {stage_q}; exit $__ec",
            timeout=300,
        )
        cleaned_inline = True
        if res.exit_code != 0:
            raise RuntimeError(
                f"Assembling upload to {guest_path} failed in VM {vm_id} "
                f"(exit {res.exit_code}): {res.stderr or res.stdout}"
            )
    finally:
        if not cleaned_inline:
            with contextlib.suppress(Exception):
                await controller.exec_in_vm(vm_id, f"rm -rf {stage_q}", timeout=60)


async def _staged_download(
    controller: CapsemController,
    files: _FilesClient,
    vm_id: str,
    guest_path: str,
    *,
    max_bytes: int | None = None,
) -> bytes:
    stage_dir = _XFER_STAGE_DIR
    part_bytes = _XFER_PART_BYTES
    rel = _rel_to_stage_dir(guest_path, stage_dir)
    if rel is not None:
        try:
            direct = bytes(await files.read(rel))
            return direct[: max_bytes + 1] if max_bytes is not None else direct
        except HttpError as exc:
            if exc.status not in (400, 403, 413):
                raise
    stage = f".capsem-xfer-{uuid.uuid4().hex[:12]}"
    stage_q = shlex.quote(posixpath.join(stage_dir, stage))
    guest_q = shlex.quote(guest_path)
    split_cmd = (
        f"head -c {max_bytes + 1} -- {guest_q} | split -b {part_bytes} -d -a 6 - part."
        if max_bytes is not None
        else f"split -b {part_bytes} -d -a 6 {guest_q} part."
    )
    try:
        listing = await _exec_checked(
            controller,
            vm_id,
            f"set -o pipefail; [ -f {guest_q} ] && mkdir -p {stage_q} "
            f"&& cd {stage_q} && {split_cmd} && ls -1",
            timeout=300,
            what=f"Splitting {guest_path} for download",
        )
        names = sorted(line.strip() for line in listing.stdout.splitlines() if line.strip())
        parts: list[bytes] = []
        total = 0
        for name in names:
            chunk = bytes(await files.read(f"{stage}/{name}"))
            if max_bytes is not None and total + len(chunk) > max_bytes:
                parts.append(chunk[: max_bytes + 1 - total])
                break
            parts.append(chunk)
            total += len(chunk)
        return b"".join(parts)
    finally:
        with contextlib.suppress(Exception):
            await controller.exec_in_vm(vm_id, f"rm -rf {stage_q}", timeout=60)
