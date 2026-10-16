"""VM subinterfaces for file transfer and statistics."""

from __future__ import annotations

import contextlib
import posixpath
import re
import shlex
import uuid
from typing import TYPE_CHECKING

from pydantic import StrictStr, TypeAdapter

from . import _operations as api
from . import models
from ._transport import HttpError
from .execution import MAX_REQUEST_BODY_BYTES, ExecResult, decode_exec_output

if TYPE_CHECKING:
    from .vm import VM

_XFER_PART_BYTES = MAX_REQUEST_BODY_BYTES - 2 * 1024 * 1024
_XFER_STAGE_DIR = "/root"
_OCI_STAGE_DIR = "/workspace"
# Direct Files API eligibility check: the service's path filter silently strips
# characters outside `[A-Za-z0-9._-/]` instead of returning 400, so only
# relative paths that the service stores verbatim take the direct Files API fast
# path; all other filenames route through staged `exec` (`shlex.quote`).
_DIRECT_REL_RE = re.compile(r"[A-Za-z0-9._-]+(?:/[A-Za-z0-9._-]+)*\Z")


def _rel_to_stage_dir(guest_path: str, stage_dir: str) -> str | None:
    if ".." in guest_path.split("/"):
        return None
    norm_stage = posixpath.normpath(stage_dir)
    norm_guest = posixpath.normpath(guest_path)
    prefix = "/" if norm_stage == "/" else f"{norm_stage}/"
    if not norm_guest.startswith(prefix):
        return None
    rel = norm_guest[len(prefix) :]
    if not rel or not _DIRECT_REL_RE.match(rel):
        return None
    return rel


def _validate_transfer_args(
    path: str,
    *,
    target: models.ExecTarget | None,
    stage_dir: str | None,
    max_bytes: int | None = None,
) -> None:
    TypeAdapter(StrictStr).validate_python(path)
    if not path or path.strip("/") == "":
        raise ValueError("path must name a file")
    if max_bytes is not None and (
        isinstance(max_bytes, bool) or not isinstance(max_bytes, int) or max_bytes < 0
    ):
        raise ValueError("max_bytes must be a nonnegative integer")
    if target is not None:
        TypeAdapter(models.ExecTarget).validate_python(target)
    if stage_dir is not None:
        TypeAdapter(StrictStr).validate_python(stage_dir)
        if not posixpath.isabs(stage_dir):
            raise ValueError("stage_dir must be an absolute POSIX path")


def _check_exec_result(vm: VM, res: ExecResult, what: str) -> ExecResult:
    if res.exit_code != 0:
        stderr = decode_exec_output(res.stderr).decode("utf-8", errors="replace")
        stdout = decode_exec_output(res.stdout).decode("utf-8", errors="replace")
        vm_label = vm.id or vm.name
        raise RuntimeError(
            f"{what} failed in VM {vm_label} (exit {res.exit_code}): {stderr or stdout}"
        )
    return res


class Resource:
    def __init__(self, vm: VM) -> None:
        self._vm = vm


class Files(Resource):
    """Paths are the guest's: `/root/x` in a VM, `/workspace/x` in its container,
    or `x` relative to the workspace. `exact=True` takes a path literally,
    relative to the workspace root, even when it is absolute."""

    async def read(self, path: str, *, exact: bool = False) -> bytes:
        vm_id = await self._vm._resolve()
        return await self._vm._call(vm_id, api.download_vm_file(
            self._vm._transport, id=vm_id, path=path, exact=exact or None,
        ))

    async def write(self, path: str, data: bytes, *, exact: bool = False) -> models.UploadResponse:
        vm_id = await self._vm._resolve()
        return await self._vm._call(vm_id, api.upload_vm_file(
            self._vm._transport, id=vm_id, path=path, exact=exact or None, body=data,
        ))

    async def list(self, path: str = "", *, depth: int | None = None, exact: bool = False) -> models.FileListResponse:
        """An empty `path` lists the workspace root."""
        vm_id = await self._vm._resolve()
        return await self._vm._call(vm_id, api.list_vm_files(
            self._vm._transport, id=vm_id,
            path=path or None, depth=depth, exact=exact or None,
        ))

    def _resolve_transfer_paths(
        self,
        path: str,
        *,
        target: models.ExecTarget | None,
        stage_dir: str | None,
    ) -> tuple[str, str]:
        if stage_dir is not None:
            effective_stage = stage_dir
        elif target is models.ExecTarget.WORKLOAD or (
            target is None and self._vm._has_container is True
        ):
            effective_stage = _OCI_STAGE_DIR
        else:
            effective_stage = _XFER_STAGE_DIR
        guest_path = (
            path if posixpath.isabs(path) else f"{effective_stage.rstrip('/')}/{path}"
        )
        return effective_stage, guest_path

    async def upload(
        self,
        path: str,
        data: bytes,
        *,
        target: models.ExecTarget | None = None,
        stage_dir: str | None = None,
    ) -> None:
        """Upload `data` to `path` in the guest, staging multi-part chunks or
        out-of-workspace paths when a single Files API write is insufficient."""
        _validate_transfer_args(path, target=target, stage_dir=stage_dir)
        if not isinstance(data, bytes):
            raise TypeError("data must be bytes")
        effective_stage_dir, guest_path = self._resolve_transfer_paths(
            path, target=target, stage_dir=stage_dir
        )
        dest_q = shlex.quote(guest_path)
        parent_q = shlex.quote(posixpath.dirname(guest_path) or "/")
        if not data:
            _check_exec_result(
                self._vm,
                await self._vm.exec(
                    f"mkdir -p {parent_q} && : > {dest_q}",
                    timeout_secs=60,
                    target=target,
                ),
                "Empty upload",
            )
            return
        part_bytes = _XFER_PART_BYTES
        rel = _rel_to_stage_dir(guest_path, effective_stage_dir)
        if rel is not None and len(data) <= part_bytes:
            try:
                await self.write(rel, data)
                return
            except HttpError as exc:
                if exc.status not in (400, 403, 413):
                    raise
        stage = f".capsem-xfer-{uuid.uuid4().hex[:12]}"
        stage_q = shlex.quote(posixpath.join(effective_stage_dir, stage))
        cleaned_inline = False
        try:
            for index, offset in enumerate(range(0, len(data), part_bytes)):
                part = data[offset : offset + part_bytes]
                await self.write(f"{stage}/{index:06d}", part)
            res = await self._vm.exec(
                f"mkdir -p {parent_q} && cat {stage_q}/* > {dest_q}; "
                f"__ec=$?; rm -rf {stage_q}; exit $__ec",
                timeout_secs=300,
                target=target,
            )
            cleaned_inline = True
            _check_exec_result(self._vm, res, f"Assembling upload to {guest_path}")
        finally:
            if not cleaned_inline:
                with contextlib.suppress(Exception):
                    await self._vm.exec(f"rm -rf {stage_q}", timeout_secs=60, target=target)

    async def download(
        self,
        path: str,
        *,
        max_bytes: int | None = None,
        target: models.ExecTarget | None = None,
        stage_dir: str | None = None,
    ) -> bytes:
        """Download `path` from the guest, splitting payloads above the HTTP
        body limit or outside the workspace root across staged parts.

        When `max_bytes` is set, raises `ValueError` if the file exceeds
        `max_bytes` bytes (while bounding guest staging to `max_bytes + 1` bytes
        so oversized files are not split in full).
        """
        _validate_transfer_args(
            path, target=target, stage_dir=stage_dir, max_bytes=max_bytes
        )
        effective_stage_dir, guest_path = self._resolve_transfer_paths(
            path, target=target, stage_dir=stage_dir
        )
        part_bytes = _XFER_PART_BYTES
        rel = _rel_to_stage_dir(guest_path, effective_stage_dir)
        if rel is not None:
            try:
                direct = bytes(await self.read(rel))
            except HttpError as exc:
                if exc.status not in (400, 403, 413):
                    raise
            else:
                if max_bytes is not None and len(direct) > max_bytes:
                    raise ValueError(f"File {guest_path} exceeds max_bytes={max_bytes}")
                return direct
        stage = f".capsem-xfer-{uuid.uuid4().hex[:12]}"
        stage_q = shlex.quote(posixpath.join(effective_stage_dir, stage))
        guest_q = shlex.quote(guest_path)
        if max_bytes is not None:
            split_cmd = (
                f"if command -v split >/dev/null 2>&1; then "
                f"head -c {max_bytes + 1} -- {guest_q} | split -b {part_bytes} -a 6 - part.; "
                f"else head -c {max_bytes + 1} {guest_q} > part.000000 && "
                f"{{ [ -s part.000000 ] || rm -f part.000000; }}; fi"
            )
        else:
            split_cmd = (
                f"if command -v split >/dev/null 2>&1; then "
                f"split -b {part_bytes} -a 6 {guest_q} part.; "
                f"else cat {guest_q} > part.000000 && "
                f"{{ [ -s part.000000 ] || rm -f part.000000; }}; fi"
            )
        try:
            listing = _check_exec_result(
                self._vm,
                await self._vm.exec(
                    f"[ -f {guest_q} ] && [ -r {guest_q} ] && mkdir -p {stage_q} "
                    f"&& cd {stage_q} && {split_cmd} && ls -1",
                    timeout_secs=300,
                    target=target,
                ),
                f"Splitting {guest_path} for download",
            )
            stdout = decode_exec_output(listing.stdout).decode("utf-8", errors="replace")
            names = sorted(line.strip() for line in stdout.splitlines() if line.strip())
            parts: list[bytes] = []
            total = 0
            for name in names:
                chunk = bytes(await self.read(f"{stage}/{name}"))
                total += len(chunk)
                if max_bytes is not None and total > max_bytes:
                    raise ValueError(f"File {guest_path} exceeds max_bytes={max_bytes}")
                parts.append(chunk)
            return b"".join(parts)
        finally:
            with contextlib.suppress(Exception):
                await self._vm.exec(f"rm -rf {stage_q}", timeout_secs=60, target=target)


class Stats(Resource):
    async def summary(self) -> models.VmStatsSummaryResponse:
        vm_id = await self._vm._resolve()
        return await self._vm._call(vm_id, api.get_vm_stats_summary(self._vm._transport, id=vm_id))

    async def details(self) -> models.VmStatsDetailResponse:
        vm_id = await self._vm._resolve()
        return await self._vm._call(vm_id, api.get_vm_stats_detail(self._vm._transport, id=vm_id))
