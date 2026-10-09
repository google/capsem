"""OCI workload container staging, healthcheck polling, and working_dir probing."""

from __future__ import annotations

import asyncio
import gzip
import io
import os
import re
import shlex
import tarfile
from collections.abc import Sequence
from pathlib import Path
from typing import Any

from .compose_fields import (
    _NAMED_VOL_RE,
    CAPSEM_INSPECT_ALLOWED_HOST_PATHS_VAR,
    _is_host_path_allowed,
    _looks_like_host_bind_source,
)
from .controller import ContainerController
from .spec import ContainerSpec

_MAX_BIND_MOUNT_BYTES = 256 * 1024 * 1024
_DURATION_RE = re.compile(r"(\d+(?:\.\d+)?)(ms|s|m|h)")
_DURATION_MULTIPLIERS = {"ms": 0.001, "s": 1.0, "m": 60.0, "h": 3600.0}


def _reset_tarinfo_ownership(ti: tarfile.TarInfo) -> tarfile.TarInfo:
    ti.uid = ti.gid = 0
    ti.uname = ti.gname = ""
    return ti


def _pack_directory_tar_gz(src_dir: Path) -> bytes:
    total = 0
    buf = io.BytesIO()
    with (
        gzip.GzipFile(filename="", mode="wb", fileobj=buf, compresslevel=1, mtime=0) as gz,
        tarfile.open(fileobj=gz, mode="w") as tf,
    ):
        for root, dirs, files in os.walk(src_dir, followlinks=False):
            dirs.sort()
            files.sort()
            root_path = Path(root)
            for name in (*dirs, *files):
                full = root_path / name
                st = full.lstat()
                if (st.st_mode & 0o170000) == 0o100000:
                    total += st.st_size
                    if total > _MAX_BIND_MOUNT_BYTES:
                        raise ValueError(
                            f"Host bind mount directory {src_dir} exceeds maximum size "
                            f"({_MAX_BIND_MOUNT_BYTES} bytes)."
                        )
                rel = full.relative_to(src_dir).as_posix()
                tf.add(full, arcname=rel, recursive=False, filter=_reset_tarinfo_ownership)
    return buf.getvalue()


async def _stage_oci_bind_volumes(
    controller: ContainerController,
    vm_id: str,
    volumes: Sequence[str],
    *,
    allowed_host_paths: Sequence[str] = (),
) -> None:
    """Upload host bind-mount files/directories or stage declared named volumes in the container."""
    for idx, vol in enumerate(volumes):
        parts = vol.split(":")
        if len(parts) < 2:
            raise ValueError(
                f"Named or invalid Compose volume {vol!r} is not supported in Capsem "
                "OCI-workload mode."
            )
        guest_dst = parts[1].strip()
        if not guest_dst.startswith("/"):
            raise ValueError(f"Container volume target path must be absolute, got {guest_dst!r}")
        if not _looks_like_host_bind_source(parts[0]):
            if not _NAMED_VOL_RE.match(parts[0]):
                raise ValueError(
                    f"Named or invalid Compose volume {vol!r} is not supported in Capsem "
                    "OCI-workload mode."
                )
            mode_cmd = (
                "chmod a+rx" if (len(parts) >= 3 and "ro" in parts[2].split(",")) else "chmod a+rwx"
            )
            q_dst = shlex.quote(guest_dst)
            res = await controller.exec_in_vm(
                vm_id, f"mkdir -p {q_dst} && ({mode_cmd} {q_dst} 2>/dev/null || true)"
            )
            if res.exit_code != 0:
                raise RuntimeError(
                    f"Failed staging named volume {parts[0]!r} at {guest_dst}: "
                    f"{res.stderr or res.stdout}"
                )
            continue
        host_src = Path(parts[0]).expanduser()
        if not _is_host_path_allowed(host_src, allowed_host_paths):
            raise ValueError(
                f"Host bind mount source {parts[0]!r} (resolved to "
                f"{str(host_src.resolve())!r}) is not in {CAPSEM_INSPECT_ALLOWED_HOST_PATHS_VAR} "
                "/ allowed_host_paths."
            )
        resolved = host_src.resolve()
        if not resolved.exists():
            raise FileNotFoundError(f"Host bind mount source not found: {resolved}")
        if resolved.is_dir():
            tmp_tar = f"/tmp/.capsem_bind_{idx}.tar.gz"
            await controller.upload_to_vm(vm_id, tmp_tar, _pack_directory_tar_gz(resolved))
            q_dst, q_tar = shlex.quote(guest_dst), shlex.quote(tmp_tar)
            res = await controller.exec_in_vm(
                vm_id,
                f"mkdir -p {q_dst} && tar -xzf {q_tar} -C {q_dst}; "
                f"__ec=$?; rm -f {q_tar}; exit $__ec",
            )
            if res.exit_code != 0:
                raise RuntimeError(
                    f"Failed extracting bind mount into {guest_dst}: {res.stderr or res.stdout}"
                )
        else:
            st = resolved.stat()
            if st.st_size > _MAX_BIND_MOUNT_BYTES:
                raise ValueError(
                    f"Host bind mount file {resolved} exceeds maximum size "
                    f"({_MAX_BIND_MOUNT_BYTES} bytes)."
                )
            data = resolved.read_bytes()
            if (parent := str(Path(guest_dst).parent)) and parent != "/":
                mk_res = await controller.exec_in_vm(vm_id, f"mkdir -p {shlex.quote(parent)}")
                if mk_res.exit_code != 0:
                    raise RuntimeError(
                        f"Failed creating parent directory {parent}: "
                        f"{mk_res.stderr or mk_res.stdout}"
                    )
            await controller.upload_to_vm(vm_id, guest_dst, data)
            mode_oct = f"{st.st_mode & 0o777:o}"
            ch_res = await controller.exec_in_vm(
                vm_id, f"chmod {mode_oct} {shlex.quote(guest_dst)}"
            )
            if ch_res.exit_code != 0:
                raise RuntimeError(
                    f"Failed setting mode {mode_oct} on {guest_dst}: "
                    f"{ch_res.stderr or ch_res.stdout}"
                )


def _parse_compose_duration(val: Any, default_secs: float) -> float:
    if isinstance(val, (int, float)):
        return max(0.1, float(val))
    if isinstance(val, str) and (matches := list(_DURATION_RE.finditer(val.strip()))):
        return max(0.1, sum(float(m.group(1)) * _DURATION_MULTIPLIERS[m.group(2)] for m in matches))
    return default_secs


async def _wait_for_oci_healthcheck(
    controller: ContainerController, vm_id: str, healthcheck: dict[str, Any]
) -> None:
    test = healthcheck.get("test")
    if isinstance(test, (list, tuple)) and len(test) >= 2:
        kind = str(test[0]).upper()
        hc_cmd = (
            str(test[1]) if kind == "CMD-SHELL" else " ".join(shlex.quote(str(x)) for x in test[1:])
        )
    elif isinstance(test, str) and test.strip():
        hc_cmd = test.strip()
    else:
        return

    interval = _parse_compose_duration(healthcheck.get("interval"), 2.0)
    timeout = max(1, int(_parse_compose_duration(healthcheck.get("timeout"), 10.0)))
    start_period = _parse_compose_duration(healthcheck.get("start_period"), 0.0)
    retries_raw = healthcheck.get("retries")
    retries = (
        max(1, int(retries_raw))
        if isinstance(retries_raw, (int, str)) and str(retries_raw).strip().isdigit()
        else 3
    )
    total_attempts = retries + max(0, int(start_period / max(interval, 0.5)))

    probe_cmd, last_err = f"sh -c {shlex.quote(hc_cmd)}", ""
    for attempt in range(total_attempts):
        res = await controller.exec_in_vm(vm_id, probe_cmd, timeout=timeout)
        if res.exit_code == 0:
            return
        last_err = (res.stderr or res.stdout).strip()
        if attempt + 1 < total_attempts:
            await asyncio.sleep(min(interval, 2.0))
    raise RuntimeError(
        f"Container in VM {vm_id} failed healthcheck ({hc_cmd!r}) "
        f"after {total_attempts} attempts: {last_err}"
    )


async def prepare_oci_workload_container(
    controller: ContainerController, vm_id: str, spec: ContainerSpec
) -> None:
    """Prepare an OCI workload container created via `Hypervisor.create(image=...)`."""
    if spec.volumes:
        await _stage_oci_bind_volumes(
            controller, vm_id, spec.volumes, allowed_host_paths=spec.allowed_host_paths
        )
    if spec.working_dir_explicit and spec.working_dir and spec.working_dir != "/":
        q_wd = shlex.quote(spec.working_dir)
        await controller.exec_in_vm(
            vm_id, f"mkdir -p {q_wd} && (chmod a+rx {q_wd} 2>/dev/null || true)"
        )
    if spec.healthcheck:
        await _wait_for_oci_healthcheck(controller, vm_id, spec.healthcheck)


async def resolve_container_working_dir(controller: ContainerController, vm_id: str) -> str:
    """Probe the active container's initial `WORKDIR` via `pwd` (falling back to `/`)."""
    res = await controller.exec_in_vm(vm_id, "pwd", timeout=15)
    pwd_out = res.stdout.strip().splitlines()
    return pwd_out[-1] if (res.exit_code == 0 and pwd_out and pwd_out[-1].startswith("/")) else "/"
