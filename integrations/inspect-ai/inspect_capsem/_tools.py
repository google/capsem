"""Inspect sandbox tools binary resolution, baking, and file transfer helpers."""

from __future__ import annotations

import contextlib
import logging
import os
import platform
import posixpath
import re
import shlex
import uuid
from pathlib import Path
from typing import Any

import inspect_capsem._controller as _ctrl_mod
from inspect_capsem._controller import CapsemController, CommandResult, _rel_to_stage_dir
from inspect_capsem.containers.dockerfile import is_root_user_spec
from inspect_capsem.containers.runtime import (
    _OCI_RUNC_EXEC_ROOT,
    _OCI_WORKLOAD_CONTAINER_ID,
)

logger = logging.getLogger(__name__)

INSPECT_SANDBOX_TOOLS_GUEST_DIR = "/var/tmp/sandbox-services"
INSPECT_SANDBOX_TOOLS_GUEST_PATH = f"{INSPECT_SANDBOX_TOOLS_GUEST_DIR}/inspect-sandbox-tools"
# Mirrors `SANDBOX_TOOLS_DIR` (`inspect_ai/util/_sandbox/_cli.py:31`) in `inspect_ai`'s
# `inspect-sandbox-tools` onedir wrapper so pre-extracting v29+ `.tar.gz` archives avoids
# per-invocation extraction overhead. If a future `inspect_ai` build changes this directory
# hash, the binary falls back to self-extracting on first invocation.
INSPECT_SANDBOX_TOOLS_ONEDIR_PATH = "/var/tmp/.da7be258e003d428"


def resolve_inspect_sandbox_tools_host_binary(
    override_path: str | Path | None = None,
) -> Path | None:
    """Locate the host `inspect-sandbox-tools` binary matching the guest architecture."""
    if override_path is not None:
        candidate = Path(override_path)
        return candidate if candidate.is_file() else None

    env_override = os.environ.get("INSPECT_CAPSEM_SANDBOX_TOOLS_PATH")
    if env_override:
        candidate = Path(env_override)
        if candidate.is_file():
            return candidate

    try:
        import inspect_ai

        arch = "arm64" if platform.machine().lower() in ("aarch64", "arm64") else "amd64"
        binaries_dir = Path(inspect_ai.__file__).resolve().parent / "binaries"
        exact = binaries_dir / f"inspect-sandbox-tools-{arch}-linux"
        if exact.is_file():
            return exact

        candidates = [
            p
            for p in binaries_dir.glob(f"inspect-sandbox-tools-{arch}*")
            if p.is_file() and not p.name.endswith(".tar") and "-musl-" not in p.name
        ]
        if not candidates:
            candidates = [
                p
                for p in binaries_dir.glob(f"inspect-sandbox-tools-{arch}*")
                if p.is_file() and not p.name.endswith(".tar")
            ]
        if candidates:

            def _version_key(p: Path) -> tuple[int, str]:
                match = re.search(r"-v(\d+)$", p.name)
                return (int(match.group(1)) if match else 0, p.name)

            candidates.sort(key=_version_key, reverse=True)
            return candidates[0]
    except Exception:
        logger.debug("Failed locating inspect-sandbox-tools in inspect_ai", exc_info=True)
    return None


def _vm_stage_path() -> str:
    return posixpath.join(_ctrl_mod._XFER_STAGE_DIR, f".capsem-xfer-file-{uuid.uuid4().hex[:12]}")


_CHOWN_WARN_SENTINEL = "CAPSEM_CHOWN_FAILED"


def _chown_to_container_user_snippet(dest_q: str, default_user: str = "") -> str:
    """Return a POSIX shell snippet that chowns `dest_q` to `default_user`, `$1`, or `/proc/1` owner if non-root."""
    default_q = shlex.quote(default_user)
    return (
        f"u={default_q}; "
        '[ -n "$u" ] || u="${1:-}"; '
        'if [ -z "$u" ] && [ -d /proc/1 ]; then '
        "  u=$(stat -c '%u:%g' /proc/1 2>/dev/null || true); "
        "fi; "
        'if [ -n "$u" ] && [ "$u" != "0" ] && [ "$u" != "root" ] '
        '&& [ "$u" != "0:0" ] && [ "$u" != "root:root" ]; then '
        f'  case "$u" in *:*) chown "$u" {dest_q} ;; '
        f'  *) chown "$u:" {dest_q} 2>/dev/null || chown "$u" {dest_q} ;; esac; '
        "fi"
    )


async def _upload_via_transfer(
    controller: Any,
    vm_id: str,
    container_id: str | None,
    dest_guest_path: str,
    data: bytes,
    *,
    container_user: str = "",
) -> None:
    """Write `data` to `dest_guest_path` in the VM, or in `container_id` inside it."""
    if not container_id:
        await controller.upload_to_vm(vm_id, dest_guest_path, data)
        return
    if container_id == _OCI_WORKLOAD_CONTAINER_ID:
        rel_ws = _rel_to_stage_dir(dest_guest_path, "/workspace")
        if rel_ws is not None:
            await controller.upload_to_vm(
                vm_id, posixpath.join(_ctrl_mod._XFER_STAGE_DIR, rel_ws), data
            )
            if not is_root_user_spec(container_user):
                dest_q = shlex.quote(dest_guest_path)
                chown_cmd = _chown_to_container_user_snippet(dest_q, default_user=container_user)
                chown_res = await controller.exec_in_vm(
                    vm_id, f"{_OCI_RUNC_EXEC_ROOT} sh -c {shlex.quote(chown_cmd)}", timeout=60
                )
                if chown_res.exit_code != 0:
                    logger.warning(
                        "Failed to chown %s in container %s: %s",
                        dest_guest_path,
                        container_id,
                        chown_res.stderr.strip() or chown_res.stdout.strip(),
                    )
            return
    vm_stage = _vm_stage_path()
    try:
        await controller.upload_to_vm(vm_id, vm_stage, data)
        dest_q = shlex.quote(dest_guest_path)
        chown_snippet = _chown_to_container_user_snippet(dest_q, default_user=container_user)
        inner = (
            f"mkdir -p {shlex.quote(posixpath.dirname(dest_guest_path) or '/')} && "
            f"cat > {dest_q} || exit $?; "
            f"({chown_snippet}) || echo '{_CHOWN_WARN_SENTINEL}' >&2"
        )
        if container_id == _OCI_WORKLOAD_CONTAINER_ID:
            cmd = f"{_OCI_RUNC_EXEC_ROOT} sh -c {shlex.quote(inner)} < {shlex.quote(vm_stage)}"
        else:
            cid_q = shlex.quote(container_id)
            inspect_u = (
                f"\"$(docker inspect -f '{{{{.Config.User}}}}' {cid_q} 2>/dev/null || true)\""
            )
            cmd = (
                f"docker exec -i -u 0 {cid_q} sh -c {shlex.quote(inner)} "
                f"-- {inspect_u} < {shlex.quote(vm_stage)}"
            )
        res = await controller.exec_in_vm(vm_id, cmd, timeout=300)
        if res.exit_code != 0:
            msg = f"Failed writing {dest_guest_path} in container {container_id}: {res.stderr}"
            raise RuntimeError(msg)
        if _CHOWN_WARN_SENTINEL in res.stderr:
            logger.warning(
                "Failed to chown %s in container %s: %s",
                dest_guest_path,
                container_id,
                res.stderr.replace(_CHOWN_WARN_SENTINEL, "").strip(),
            )
    finally:
        with contextlib.suppress(Exception):
            await controller.exec_in_vm(vm_id, f"rm -f {shlex.quote(vm_stage)}", timeout=60)


async def _download_via_transfer(
    controller: Any, vm_id: str, container_id: str | None, src_guest_path: str
) -> bytes:
    """Read `src_guest_path` from the VM, or from `container_id` inside it."""
    if not container_id:
        return await controller.download_from_vm(vm_id, src_guest_path)
    if container_id == _OCI_WORKLOAD_CONTAINER_ID:
        rel_ws = _rel_to_stage_dir(src_guest_path, "/workspace")
        if rel_ws is not None:
            return await controller.download_from_vm(
                vm_id, posixpath.join(_ctrl_mod._XFER_STAGE_DIR, rel_ws)
            )
    vm_stage = _vm_stage_path()
    try:
        exec_prefix = (
            _OCI_RUNC_EXEC_ROOT
            if container_id == _OCI_WORKLOAD_CONTAINER_ID
            else f"docker exec -u 0 {shlex.quote(container_id)}"
        )
        res = await controller.exec_in_vm(
            vm_id,
            f"{exec_prefix} cat {shlex.quote(src_guest_path)} > {shlex.quote(vm_stage)}",
            timeout=300,
        )
        if res.exit_code != 0:
            msg = f"Failed reading {src_guest_path} in container {container_id}: {res.stderr}"
            raise PermissionError(msg)
        return await controller.download_from_vm(vm_id, vm_stage)
    finally:
        with contextlib.suppress(Exception):
            await controller.exec_in_vm(vm_id, f"rm -f {shlex.quote(vm_stage)}", timeout=60)


def _onedir_extract_command() -> str:
    onedir_q = shlex.quote(INSPECT_SANDBOX_TOOLS_ONEDIR_PATH)
    guest_path_q = shlex.quote(INSPECT_SANDBOX_TOOLS_GUEST_PATH)
    return (
        f"mkdir -p {onedir_q} && "
        f"tar -xzf {guest_path_q} -C {onedir_q} && "
        f"(chown -R root:root {onedir_q} 2>/dev/null || true) && "
        f"chmod -R 700 {onedir_q}"
    )


async def bake_sandbox_tools_into_controller(
    controller: CapsemController,
    vm_id: str,
    *,
    container_id: str | None = None,
    host_binary_path: str | Path | None = None,
) -> bool:
    """Install `inspect-sandbox-tools` into `/var/tmp/sandbox-services/` (0700 root:root)."""
    binary = resolve_inspect_sandbox_tools_host_binary(host_binary_path)
    if binary is None or not binary.is_file():
        logger.warning(
            "Could not locate host inspect-sandbox-tools binary to bake into VM %s", vm_id
        )
        return False

    async def _run_target(cmd: str) -> CommandResult:
        if container_id == _OCI_WORKLOAD_CONTAINER_ID:
            return await controller.exec_in_vm(
                vm_id, f"{_OCI_RUNC_EXEC_ROOT} sh -c {shlex.quote(cmd)}", timeout=60
            )
        if container_id:
            return await controller.exec_in_vm(
                vm_id,
                f"docker exec -u 0 {shlex.quote(container_id)} bash -c {shlex.quote(cmd)}",
                timeout=60,
            )
        return await controller.exec_in_vm(vm_id, cmd, timeout=60)

    if (
        await _run_target(f"test -x {shlex.quote(INSPECT_SANDBOX_TOOLS_GUEST_PATH)}")
    ).exit_code == 0:
        return True

    raw = binary.read_bytes()
    guest_dir_q = shlex.quote(INSPECT_SANDBOX_TOOLS_GUEST_DIR)
    guest_path_q = shlex.quote(INSPECT_SANDBOX_TOOLS_GUEST_PATH)
    extract_suffix = f" && {_onedir_extract_command()}" if raw[:2] == b"\x1f\x8b" else ""
    if container_id:
        vm_stage = _vm_stage_path()
        await controller.upload_to_vm(vm_id, vm_stage, raw)
        inner_install = (
            f"mkdir -p {guest_dir_q} && "
            f"cat > {guest_path_q} && "
            f"chmod 700 {guest_path_q}{extract_suffix}"
        )
        exec_prefix = (
            _OCI_RUNC_EXEC_ROOT
            if container_id == _OCI_WORKLOAD_CONTAINER_ID
            else f"docker exec -i -u 0 {shlex.quote(container_id)}"
        )
        res = await controller.exec_in_vm(
            vm_id,
            f"{exec_prefix} sh -c {shlex.quote(inner_install)} < {shlex.quote(vm_stage)}; "
            f"rc=$?; rm -f {shlex.quote(vm_stage)}; exit $rc",
            timeout=300,
        )
    else:
        await controller.upload_to_vm(vm_id, INSPECT_SANDBOX_TOOLS_GUEST_PATH, raw)
        res = await controller.exec_in_vm(
            vm_id, f"chmod 700 {guest_path_q}{extract_suffix}", timeout=120
        )
    if res.exit_code != 0:
        logger.warning("Failed baking inspect-sandbox-tools into VM %s: %s", vm_id, res.stderr)
        return False
    return True
