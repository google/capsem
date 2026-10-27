"""File, tar-archive transfer, and container definition helpers for `CapsemEnvironment`."""

from __future__ import annotations

import asyncio
import atexit
import contextlib
import os
import shlex
import shutil
import tempfile
import uuid
from collections.abc import Generator, Sequence
from pathlib import Path
from typing import TYPE_CHECKING, Any

from capsem_integrations import containers
from capsem_integrations._exec import _resolve_guest_path
from capsem_integrations.containers.build_grant import (
    default_host_build_policy,
    resolve_effective_host_build,
)
from capsem_integrations.containers.compose_fields import _realpath
from harbor.environments.definition import (
    DOCKERFILE_NAME,
    require_agent_environment_definition,
    should_use_prebuilt_docker_image,
)
from harbor.environments.tar_transfer import (
    extract_dir_from_bytes,
    pack_dir_to_bytes,
    remote_pack_command,
)

if TYPE_CHECKING:
    from capsem_integrations._controller import CapsemController

_INIT_TEARDOWN_TIMEOUT_SECS = 25.0
_TAR_TRANSFER_TIMEOUT_SEC = 300
_COMPOSE_FILENAMES = ("docker-compose.yaml", "docker-compose.yml", "compose.yaml", "compose.yml")


async def _start_vm_shielded(
    controller: CapsemController,
    *,
    cpu_count: int,
    ram_gb: int,
    image: str | None,
    **extra_kw: Any,
) -> str:
    """Start a VM under `asyncio.shield` and stop any created VM if cancelled mid-flight."""
    start_task = asyncio.ensure_future(
        controller.start_vm(cpu_count=cpu_count, ram_gb=ram_gb, image=image, **extra_kw)
    )
    try:
        return await asyncio.shield(start_task)
    except asyncio.CancelledError:
        vm_id = ""
        with contextlib.suppress(BaseException):
            vm_id = await asyncio.wait_for(
                asyncio.shield(start_task), timeout=_INIT_TEARDOWN_TIMEOUT_SECS
            )
        if vm_id:
            with contextlib.suppress(Exception):
                await asyncio.wait_for(
                    asyncio.shield(controller.stop_vm(vm_id)), timeout=_INIT_TEARDOWN_TIMEOUT_SECS
                )
        else:
            start_task.cancel()
            with contextlib.suppress(BaseException):
                await start_task
        raise


@contextlib.contextmanager
def _harbor_build_policy(
    environment_dir: Path,
    allowed_host_paths: Sequence[str],
    host_build: Any,
    *,
    allow_internet: bool = True,
) -> Generator[None, None, None]:
    """Bind per-trial build policy in `contextvars` without mutating `os.environ`."""
    roots = (
        _realpath(environment_dir),
        *(_realpath(p) for p in allowed_host_paths if p and p.strip()),
    )
    with default_host_build_policy(roots, host_build, allow_internet=allow_internet):
        yield


def _validate_environment_definition(
    environment_dir: Path,
    *,
    execution_mode: str,
    docker_image: str | None,
    allowed_host_paths: Sequence[str],
    allowed_host_env: Sequence[str],
    host_build: Any,
    allow_internet: bool = True,
) -> tuple[Path | None, dict[str, Any]]:
    compose_file = next(
        (p for n in _COMPOSE_FILENAMES if (p := environment_dir / n).is_file()), None
    )
    if compose_file is not None:
        with _harbor_build_policy(
            environment_dir, allowed_host_paths, host_build, allow_internet=allow_internet
        ):
            parsed = containers.parse_host_compose_yaml_file(
                compose_file, allowed_host_env=allowed_host_env
            )
            fields = containers.extract_capsem_compose_fields(
                parsed,
                base_dir=compose_file.parent,
                allowed_host_env=allowed_host_env,
                allowed_host_paths=allowed_host_paths,
                host_build=host_build,
            )
        return compose_file, fields
    if execution_mode == "container":
        require_agent_environment_definition(environment_dir, docker_image=docker_image)
    return None, {}


def _resolve_container_start_inputs(
    environment_dir: Path,
    *,
    execution_mode: str,
    docker_image: str | None,
    force_build: bool,
    compose_fields: dict[str, Any],
    allowed_host_paths: Sequence[str],
    host_build: Any,
    allow_internet: bool = True,
) -> tuple[str | None, dict[str, Any] | None, tuple[str, ...] | None]:
    if execution_mode == "vm":
        return None, None, None
    oci_image, build_spec = None, None
    use_prebuilt = should_use_prebuilt_docker_image(
        environment_dir, docker_image=docker_image, force_build=force_build
    )
    df_path = environment_dir / DOCKERFILE_NAME
    with _harbor_build_policy(
        environment_dir, allowed_host_paths, host_build, allow_internet=allow_internet
    ):
        if compose_fields:
            oci_image, build_spec = compose_fields.get("image"), compose_fields.get("build")
        elif use_prebuilt and df_path.is_file() and host_build is not False and docker_image:
            p_dir = Path(
                tempfile.mkdtemp(prefix=f"capsem-harbor-prebuilt-{os.getuid()}-")
            ).resolve()
            atexit.register(shutil.rmtree, p_dir, True)
            p_df = p_dir / DOCKERFILE_NAME
            p_df.write_text(f"FROM {docker_image}\n", encoding="utf-8")
            prebuilt_paths = (*allowed_host_paths, str(p_dir))
            with _harbor_build_policy(
                environment_dir, prebuilt_paths, host_build, allow_internet=allow_internet
            ):
                build_spec = resolve_effective_host_build(
                    str(p_df),
                    str(p_dir),
                    stanza="Dockerfile",
                    allowed_host_paths=prebuilt_paths,
                    host_build=host_build,
                )
        elif use_prebuilt:
            oci_image = docker_image
        elif df_path.is_file():
            build_spec = resolve_effective_host_build(
                str(df_path),
                str(environment_dir),
                stanza="Dockerfile",
                allowed_host_paths=allowed_host_paths,
                host_build=host_build,
            )
        else:
            oci_image = docker_image
    raw_cmd = compose_fields.get("command")
    if isinstance(raw_cmd, str) and raw_cmd.strip():
        return oci_image, build_spec, tuple(shlex.split(raw_cmd))
    if isinstance(raw_cmd, (list, tuple)) and raw_cmd:
        return oci_image, build_spec, tuple(str(x) for x in raw_cmd)
    return oci_image, build_spec, None


async def upload_file(
    controller: CapsemController,
    vm_id: str,
    source_path: Path | str,
    target_path: str,
    *,
    working_dir: str = "/workspace",
) -> None:
    """Upload a local file to `target_path` in the guest VM or OCI workload container."""
    src = Path(source_path)
    if not src.is_file():
        raise FileNotFoundError(f"Source file does not exist: {source_path}")
    guest_dst = _resolve_guest_path(target_path, working_dir)
    data = await asyncio.to_thread(src.read_bytes)
    await controller.upload_to_vm(vm_id, guest_dst, data)
    mode = src.stat().st_mode & 0o777
    if mode & 0o111:
        res = await controller.exec_in_vm(
            vm_id, f"chmod {mode:o} {shlex.quote(guest_dst)}", timeout=30
        )
        if res.exit_code != 0:
            raise RuntimeError(
                f"Failed to set permissions {mode:o} on {guest_dst!r}: {res.stderr or res.stdout}"
            )


async def download_file(
    controller: CapsemController,
    vm_id: str,
    source_path: str,
    target_path: Path | str,
    *,
    working_dir: str = "/workspace",
) -> None:
    """Download `source_path` from the guest VM or OCI workload container to `target_path`."""
    guest_src = _resolve_guest_path(source_path, working_dir)
    data = await controller.download_from_vm(vm_id, guest_src)
    dst = Path(target_path)
    dst.parent.mkdir(parents=True, exist_ok=True)
    await asyncio.to_thread(dst.write_bytes, data)


async def upload_dir(
    controller: CapsemController,
    vm_id: str,
    source_dir: Path | str,
    target_dir: str,
    *,
    working_dir: str = "/workspace",
) -> None:
    """Pack `source_dir` as `.tar.gz`, upload to `/var/tmp`, and extract into `target_dir`."""
    guest_dst = _resolve_guest_path(target_dir, working_dir)
    buffer = await asyncio.to_thread(pack_dir_to_bytes, source_dir, compress=True)
    remote_tar = f"/var/tmp/.hb-upload-{uuid.uuid4().hex[:12]}.tar.gz"
    await controller.upload_to_vm(vm_id, remote_tar, buffer.getvalue())
    q_dst, q_tar = shlex.quote(guest_dst), shlex.quote(remote_tar)
    unpack_cmd = (
        f"mkdir -p {q_dst} && tar -xozf {q_tar} -C {q_dst}; __ec=$?; rm -f {q_tar}; exit $__ec"
    )
    res = await controller.exec_in_vm(vm_id, unpack_cmd, timeout=_TAR_TRANSFER_TIMEOUT_SEC)
    if res.exit_code != 0:
        raise RuntimeError(
            f"Failed to extract upload archive into {guest_dst!r} "
            f"(exit {res.exit_code}): {res.stderr or res.stdout}"
        )


async def download_dir(
    controller: CapsemController,
    vm_id: str,
    source_dir: str,
    target_dir: Path | str,
    *,
    working_dir: str = "/workspace",
) -> None:
    """Pack `source_dir` on the guest into `/var/tmp`, download, and extract into `target_dir`."""
    guest_src = _resolve_guest_path(source_dir, working_dir)
    remote_tar = f"/var/tmp/.hb-download-{uuid.uuid4().hex[:12]}.tar.gz"
    pack_cmd = remote_pack_command(guest_src, remote_tar)
    res = await controller.exec_in_vm(vm_id, pack_cmd, timeout=_TAR_TRANSFER_TIMEOUT_SEC)
    if res.exit_code != 0:
        raise RuntimeError(
            f"Failed to create download archive for {guest_src!r} "
            f"(exit {res.exit_code}): {res.stderr or res.stdout}"
        )
    try:
        data = await controller.download_from_vm(vm_id, remote_tar)
    finally:
        with contextlib.suppress(Exception):
            await controller.exec_in_vm(vm_id, f"rm -f {shlex.quote(remote_tar)}", timeout=30)
    await asyncio.to_thread(extract_dir_from_bytes, data, target_dir)


__all__ = [
    "_INIT_TEARDOWN_TIMEOUT_SECS",
    "_harbor_build_policy",
    "_resolve_container_start_inputs",
    "_resolve_guest_path",
    "_start_vm_shielded",
    "_validate_environment_definition",
    "download_dir",
    "download_file",
    "upload_dir",
    "upload_file",
]
