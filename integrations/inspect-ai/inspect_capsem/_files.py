"""Guest file read, write, validation, and UTF-8 decoding helpers for Capsem sandboxes."""

from __future__ import annotations

import posixpath
import shlex
from typing import TYPE_CHECKING

from inspect_ai.util import OutputLimitExceededError, SandboxEnvironmentLimits

if TYPE_CHECKING:
    from inspect_capsem._controller import CapsemController


def _resolve_guest_path(path: str, working_dir: str) -> str:
    """Resolve `path` against `working_dir` if relative; return normalized path."""
    if posixpath.isabs(path):
        return posixpath.normpath(path)
    return posixpath.normpath(posixpath.join(working_dir, path))


def _decode_text_bytes(raw_bytes: bytes, file: str) -> str:
    """Decode guest file bytes to UTF-8, rejecting binary / NUL characters."""
    try:
        text = raw_bytes.decode("utf-8")
    except UnicodeDecodeError as exc:
        raise UnicodeDecodeError(
            exc.encoding,
            exc.object,
            exc.start,
            exc.end,
            f"Cannot decode binary file '{file}' as UTF-8",
        ) from exc
    if "\x00" in text:
        raise UnicodeDecodeError(
            "utf-8",
            raw_bytes,
            0,
            1,
            f"File '{file}' contains NUL bytes and appears to be binary",
        )
    return text


async def read_guest_file(
    controller: CapsemController,
    vm_id: str,
    working_dir: str,
    file: str,
    *,
    text: bool = True,
) -> str | bytes:
    """Read `file` from guest VM, enforcing size limits and permissions."""
    resolved = _resolve_guest_path(file, working_dir)
    limit_bytes = SandboxEnvironmentLimits.MAX_READ_FILE_SIZE
    res_q = shlex.quote(resolved)

    stat_script = (
        f"if [ ! -e {res_q} ]; then echo 'NOT_FOUND'; exit 2; fi; "
        f"if [ -d {res_q} ]; then echo 'IS_DIR'; exit 21; fi; "
        f"if [ ! -f {res_q} ]; then echo 'NOT_REGULAR'; exit 22; fi; "
        f"mode=$(stat -c '%a' {res_q} 2>/dev/null || echo 644); "
        f"if [ $((0$mode & 0444)) -eq 0 ]; then echo 'NO_READ'; exit 13; fi; "
        f"size=$(stat -c '%s' {res_q} 2>/dev/null || echo 0); "
        f'echo "SIZE:$size"'
    )
    stat_res = await controller.exec_in_vm(vm_id, stat_script, timeout=30)
    if stat_res.exit_code == 2 or "NOT_FOUND" in stat_res.stdout:
        raise FileNotFoundError(f"No such file or directory: '{file}'")
    if stat_res.exit_code == 21 or "IS_DIR" in stat_res.stdout:
        raise IsADirectoryError(f"Is a directory: '{file}'")
    if stat_res.exit_code == 22 or "NOT_REGULAR" in stat_res.stdout:
        raise PermissionError(f"Not a regular file: '{file}'")
    if stat_res.exit_code == 13 or "NO_READ" in stat_res.stdout:
        raise PermissionError(f"Permission denied: '{file}'")

    for line in stat_res.stdout.splitlines():
        if line.startswith("SIZE:"):
            if int(line.split(":", 1)[1].strip() or "0") > limit_bytes:
                raise OutputLimitExceededError(
                    limit_str=SandboxEnvironmentLimits.MAX_READ_FILE_SIZE_STR,
                    truncated_output=None,
                )
            break

    raw_bytes = await controller.download_from_vm(vm_id, resolved, max_bytes=limit_bytes)
    if len(raw_bytes) > limit_bytes:
        raise OutputLimitExceededError(
            limit_str=SandboxEnvironmentLimits.MAX_READ_FILE_SIZE_STR,
            truncated_output=None,
        )
    return _decode_text_bytes(raw_bytes, file) if text else raw_bytes


async def write_guest_file(
    controller: CapsemController,
    vm_id: str,
    working_dir: str,
    file: str,
    contents: str | bytes,
) -> None:
    """Write `contents` to `file` in guest VM, checking permissions."""
    resolved = _resolve_guest_path(file, working_dir)
    data = contents.encode("utf-8") if isinstance(contents, str) else contents
    res_q = shlex.quote(resolved)

    stat_script = (
        f"if [ -d {res_q} ]; then echo 'IS_DIR'; exit 21; fi; "
        f"if [ -e {res_q} ]; then "
        f"  mode=$(stat -c '%a' {res_q} 2>/dev/null || echo 644); "
        f"  if [ $((0$mode & 0222)) -eq 0 ]; then echo 'NO_WRITE'; exit 13; fi; "
        f"fi"
    )
    stat_res = await controller.exec_in_vm(vm_id, stat_script, timeout=30)
    if stat_res.exit_code == 21 or "IS_DIR" in stat_res.stdout:
        raise IsADirectoryError(f"Is a directory: '{file}'")
    if stat_res.exit_code == 13 or "NO_WRITE" in stat_res.stdout:
        raise PermissionError(f"Permission denied: '{file}'")

    await controller.upload_to_vm(vm_id, resolved, data)
