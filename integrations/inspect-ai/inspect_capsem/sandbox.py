"""Standalone Capsem SandboxEnvironment for Inspect AI."""

from __future__ import annotations

import asyncio
import atexit
import base64
import contextlib
import logging
import posixpath
import shlex
import uuid
from typing import TYPE_CHECKING, Any, Literal, overload

from capsem.execution import EXEC_TIMEOUT_CEILING_SECS
from inspect_ai.util import (
    ExecResult,
    OutputLimitExceededError,
    SandboxConnection,
    SandboxEnvironment,
    SandboxEnvironmentConfigType,
    SandboxEnvironmentLimits,
)

from inspect_capsem._compose import coerce_config
from inspect_capsem._controller import (
    _MANAGED_VM_PREFIX,
    CapsemController,
    CommandResult,
    SdkCapsemController,
)
from inspect_capsem._tools import (
    INSPECT_SANDBOX_TOOLS_GUEST_DIR,
    INSPECT_SANDBOX_TOOLS_GUEST_PATH,
    _download_via_transfer,
    _upload_via_transfer,
    bake_sandbox_tools_into_controller,
    resolve_inspect_sandbox_tools_host_binary,
)
from inspect_capsem.config import CapsemSandboxConfig
from inspect_capsem.containers.dockerfile import (
    _CA_ENV,
    _CONTAINER_HOME_FIX,
    is_root_user_spec,
)
from inspect_capsem.containers.runtime import (
    _OCI_RUNC_EXEC,
    _OCI_RUNC_EXEC_PREFIX,
    _OCI_RUNC_EXEC_ROOT,
    _OCI_WORKLOAD_CONTAINER_ID,
    prepare_oci_workload_container,
    start_container_for_init,
)

if TYPE_CHECKING:
    from typing import ClassVar

logger = logging.getLogger(__name__)

_TIMEOUT_SENTINEL = "__CAPSEM_INSPECT_EXEC_TIMED_OUT__"
# Service-side deadline on top of the guest `timeout`, so the guest reports the timeout.
_EXEC_TIMEOUT_MARGIN_SECS = 10
# Upper bound on tearing down a failed sample_init; stays under 30s so SIGINT exits promptly.
_INIT_TEARDOWN_TIMEOUT_SECS = 25.0
_TERMINAL_VM_STATUSES = frozenset(
    {"stopped", "exited", "failed", "error", "dead", "terminated", "crashed", "aborted", "defunct"}
)
_PROCESS_OWNED_VMS: dict[str, str | None] = {}


def _register_process_owned_vm(vm_id: str, task_name: str | None = None) -> None:
    if vm_id:
        _PROCESS_OWNED_VMS[vm_id] = task_name


def _unregister_process_owned_vm(vm_id: str) -> None:
    if vm_id:
        _PROCESS_OWNED_VMS.pop(vm_id, None)


async def _stop_vms(controller: CapsemController, vm_ids: list[str], *, reason: str) -> list[str]:
    swept: list[str] = []
    for vid in vm_ids:
        try:
            await controller.stop_vm(vid)
            _unregister_process_owned_vm(vid)
            swept.append(vid)
        except Exception as exc:
            logger.warning(
                "Failed to stop %s Capsem VM %s (run 'inspect sandbox cleanup capsem' to retry): %s",
                reason,
                vid,
                exc,
            )
    if swept:
        logger.info("Stopped %d %s Capsem VM(s): %s", len(swept), reason, swept)
    return swept


async def sweep_process_owned_vms(
    task_name: str | None = None, controller: CapsemController | None = None
) -> list[str]:
    """Stop any VMs started by this process (or by `task_name`) that have not yet been stopped."""
    if task_name is None:
        matching_ids = list(_PROCESS_OWNED_VMS.keys())
    else:
        matching_ids = [
            vid for vid, rec_task in _PROCESS_OWNED_VMS.items() if rec_task == task_name
        ]
    if not matching_ids:
        return []
    owns_controller = controller is None
    if controller is None:
        try:
            controller = SdkCapsemController()
        except Exception:
            logger.debug("Controller creation failed during process VM sweep", exc_info=True)
            return []
    try:
        return await _stop_vms(controller, matching_ids, reason="orphaned process-owned")
    finally:
        if owns_controller:
            with contextlib.suppress(Exception):
                await controller.close()


@atexit.register
def _atexit_sweep_process_owned_vms() -> None:
    if not _PROCESS_OWNED_VMS:
        return
    with contextlib.suppress(Exception):
        asyncio.run(sweep_process_owned_vms())


def _can_use_oci_image_create(cfg: CapsemSandboxConfig) -> bool:
    """Whether an image task can boot directly via `start_vm(image=...)`."""
    return not (
        cfg.execution_mode != "container"
        or not cfg.image
        or bool(cfg.dockerfile)
        or cfg.template not in ("", "default", "code")
        or bool(
            cfg.volumes
            or cfg.ports
            or cfg.expose
            or cfg.mem_limit
            or cfg.network_mode
            or cfg.user
            or cfg.init
            or cfg.healthcheck
            or cfg.entrypoint is not None
        )
    )


def _truncate_utf8(text: str, max_bytes: int) -> tuple[str, bool]:
    encoded = text.encode("utf-8", errors="replace")
    if len(encoded) <= max_bytes:
        return text, False
    truncated = encoded[-max_bytes:].decode("utf-8", errors="ignore")
    return truncated, True


def _resolve_guest_path(path: str, working_dir: str) -> str:
    if posixpath.isabs(path):
        return posixpath.normpath(path)
    return posixpath.normpath(posixpath.join(working_dir, path))


def _format_exec_command(
    exec_expr: str,
    *,
    effective_cwd: str,
    env: dict[str, str] | None,
    user: str | None,
    timeout: int | None,
) -> tuple[str, int]:
    env_prefix = ""
    if env:
        assignments = " ".join(f"{k}={shlex.quote(str(v))}" for k, v in env.items())
        env_prefix = f"export {assignments}; "
    inner_body = f"cd {shlex.quote(effective_cwd)} && {env_prefix}{exec_expr}"
    if not is_root_user_spec(user):
        assert user is not None
        target_user = user.split(":", 1)[0].strip()
        su_user = (
            f'"$(id -un {shlex.quote(target_user)})"'
            if target_user.isdigit()
            else shlex.quote(target_user)
        )
        user_env_reset = (
            '__u="$(id -un)"; '
            '__h="$(getent passwd "$(id -u)" 2>/dev/null | cut -d: -f6)"; '
            'export USER="$__u" LOGNAME="$__u" HOME="${__h:-/home/$__u}"; '
        )
        user_body = f"{user_env_reset}{inner_body}"
        inner_body = (
            f"if id -u {shlex.quote(target_user)} >/dev/null 2>&1; then "
            f"  su -m {su_user} -s /bin/bash -c {shlex.quote(user_body)}; "
            f"else "
            f'  echo "capsem: unknown user {user}" >&2; exit 1; '
            f"fi"
        )
    if timeout is None:
        return inner_body, EXEC_TIMEOUT_CEILING_SECS
    # The guest `timeout` must fire before the service's own ceiling (plus our margin).
    timeout_secs = min(int(timeout), EXEC_TIMEOUT_CEILING_SECS - _EXEC_TIMEOUT_MARGIN_SECS)
    if timeout_secs < timeout:
        logger.warning(
            "Exec timeout %ss exceeds the Capsem limit; using %ss", timeout, timeout_secs
        )
    timed_script = (
        f"SECONDS=0; "
        f"timeout -k 1s {timeout_secs}s bash -c {shlex.quote(inner_body)}; "
        f"rc=$?; "
        f"elapsed=$SECONDS; "
        f"if [ $rc -eq 124 ] || {{ [ $rc -eq 137 ] && [ $elapsed -ge {timeout_secs} ]; }}; then "
        f"  echo '{_TIMEOUT_SENTINEL}' >&2; "
        f"fi; "
        f"exit $rc"
    )
    return timed_script, max(timeout_secs + _EXEC_TIMEOUT_MARGIN_SECS, 30)


async def _teardown_failed_init(controller: CapsemController, vm_id: str) -> None:
    """Stop `vm_id` after a failed `sample_init` under `asyncio.shield` and a bounded deadline."""
    try:
        await asyncio.wait_for(
            asyncio.shield(controller.stop_vm(vm_id)), timeout=_INIT_TEARDOWN_TIMEOUT_SECS
        )
        _unregister_process_owned_vm(vm_id)
    except Exception as exc:
        logger.warning(
            "Cleanup after failed sample_init of VM %s failed (run 'inspect sandbox cleanup capsem' to retry): %s",
            vm_id,
            exc,
        )


async def sweep_leftover_vms(controller: CapsemController) -> list[str]:
    """Delete leaked `inspect-capsem-*` VMs in a terminal state; return their ids.

    Only VMs whose name starts with `inspect-capsem-` AND whose status is explicitly
    terminal (`Stopped`, `Failed`, `Exited`, etc.) and not owned by the current process
    are deleted. Running, booting, starting, or paused VMs may belong to a concurrent eval
    and are never touched. Errors are logged, never raised.
    """
    try:
        vms = await controller.list_vms()
    except Exception:
        logger.debug("list_vms failed during leftover VM sweep", exc_info=True)
        return []
    owned_ids = set(_PROCESS_OWNED_VMS.keys())
    target_ids: list[str] = []
    for vm_info in vms:
        vid = str(vm_info.get("id") or "")
        name = str(vm_info.get("name") or vid)
        status = str(vm_info.get("status") or "").strip().lower()
        if (
            vid
            and name.startswith(_MANAGED_VM_PREFIX)
            and vid not in owned_ids
            and status in _TERMINAL_VM_STATUSES
        ):
            target_ids.append(vid)
    if not target_ids:
        return []
    return await _stop_vms(controller, target_ids, reason="leftover inspect-capsem")


class CapsemSandboxEnvironment(SandboxEnvironment):
    """Spec-compliant Inspect AI `SandboxEnvironment` backed by Capsem."""

    _active_environments: ClassVar[dict[str, CapsemSandboxEnvironment]] = {}

    def __init__(
        self,
        vm_id: str,
        controller: CapsemController | None = None,
        *,
        container_id: str | None = None,
        working_dir: str = "/workspace",
        execution_mode: str = "vm",
        owns_controller: bool | None = None,
        task_name: str | None = None,
        user: str | None = None,
    ) -> None:
        super().__init__()
        created_controller = False
        if controller is None:
            resolved_controller: CapsemController = SdkCapsemController()
            created_controller = True
        else:
            resolved_controller = controller

        self._vm_id = vm_id
        self._container_id = container_id
        self._controller = resolved_controller
        self._working_dir = working_dir
        self._execution_mode = execution_mode
        self._cleaned_up = False
        self._owns_controller = created_controller if owns_controller is None else owns_controller
        self._task_name = task_name
        self._user = user
        self._instance_id = f"{vm_id}:{container_id or 'vm'}:{uuid.uuid4().hex[:6]}"
        CapsemSandboxEnvironment._active_environments[self._instance_id] = self

    @property
    def vm_id(self) -> str:
        return self._vm_id

    @property
    def container_id(self) -> str | None:
        return self._container_id

    @property
    def execution_mode(self) -> str:
        return self._execution_mode

    @classmethod
    def config_files(cls) -> list[str]:
        return [
            "compose.yaml",
            "compose.yml",
            "docker-compose.yaml",
            "docker-compose.yml",
            "Dockerfile",
            "Containerfile",
        ]

    @classmethod
    def is_docker_compatible(cls) -> bool:
        return True

    @classmethod
    def default_concurrency(cls) -> int | None:
        return 4

    @classmethod
    def config_deserialize(cls, config: dict[str, Any]) -> CapsemSandboxConfig:
        return coerce_config(config, resolve_compose=False)

    @classmethod
    async def task_init(
        cls, task_name: str, config: SandboxEnvironmentConfigType | str | None
    ) -> None:
        del task_name, config
        try:
            controller = SdkCapsemController()
        except Exception:
            logger.debug("Controller creation failed during task_init", exc_info=True)
            return
        try:
            await asyncio.wait_for(
                sweep_leftover_vms(controller), timeout=_INIT_TEARDOWN_TIMEOUT_SECS
            )
        except Exception:
            logger.debug("Leftover VM sweep failed", exc_info=True)
        finally:
            with contextlib.suppress(Exception):
                await controller.close()

    @classmethod
    async def task_cleanup(
        cls, task_name: str, config: SandboxEnvironmentConfigType | str | None, cleanup: bool
    ) -> None:
        del config
        if not cleanup:
            surviving_ids = [
                vid
                for vid, rec_task in list(_PROCESS_OWNED_VMS.items())
                if task_name is None or rec_task == task_name
            ]
            for vid in surviving_ids:
                _PROCESS_OWNED_VMS.pop(vid, None)
            matching_envs = [
                env
                for env in list(cls._active_environments.values())
                if task_name is None or env._task_name == task_name
            ]
            for env in matching_envs:
                cls._active_environments.pop(env._instance_id, None)
                if env._vm_id and env._vm_id not in surviving_ids:
                    surviving_ids.append(env._vm_id)
                if env._owns_controller:
                    env._owns_controller = False
                    with contextlib.suppress(Exception):
                        await env._controller.close()
            if surviving_ids:
                if len(surviving_ids) == 1:
                    print(
                        f"Capsem sandbox VM left running: {surviving_ids[0]}\n"
                        f"Clean up with: inspect sandbox cleanup capsem {surviving_ids[0]}"
                    )
                else:
                    joined = ", ".join(surviving_ids)
                    cleanup_lines = "\n".join(
                        f"  inspect sandbox cleanup capsem {sid}" for sid in surviving_ids
                    )
                    print(
                        f"Capsem sandbox VMs left running: {joined}\nClean up with:\n{cleanup_lines}"
                    )
            return

        matching = [
            env
            for env in list(cls._active_environments.values())
            if task_name is None or env._task_name == task_name
        ]
        for env in matching:
            with contextlib.suppress(Exception):
                await env.cleanup()
        with contextlib.suppress(Exception):
            await asyncio.wait_for(
                sweep_process_owned_vms(task_name),
                timeout=_INIT_TEARDOWN_TIMEOUT_SECS,
            )

    @classmethod
    async def _start_vm_for_sample(
        cls,
        controller: CapsemController,
        cfg: CapsemSandboxConfig,
        *,
        use_oci_image: bool,
        task_name: str,
    ) -> str:
        if use_oci_image:
            if cfg.command is None:
                oci_cmd: tuple[str, ...] = ("sleep", "infinity")
            elif isinstance(cfg.command, str):
                oci_cmd = ("sh", "-c", cfg.command)
            else:
                oci_cmd = tuple(str(a) for a in cfg.command)
            start_coro = controller.start_vm(
                template=cfg.template,
                cpu_count=cfg.cpu_count,
                ram_gb=cfg.ram_gb,
                image=cfg.image,
                command=oci_cmd,
                env={**_CA_ENV, **cfg.environment},
            )
        else:
            start_coro = controller.start_vm(
                template=cfg.template, cpu_count=cfg.cpu_count, ram_gb=cfg.ram_gb
            )
        start_task = asyncio.ensure_future(start_coro)
        try:
            vid = await asyncio.shield(start_task)
        except asyncio.CancelledError:
            # Best-effort bounded wait to stop a VM that finishes create() right as cancel arrives;
            # VMs that finish booting later on the gateway are reaped by `inspect sandbox cleanup capsem`.
            with contextlib.suppress(BaseException):
                vid = await asyncio.wait_for(
                    asyncio.shield(start_task), timeout=_INIT_TEARDOWN_TIMEOUT_SECS
                )
                try:
                    await controller.stop_vm(vid)
                except Exception:
                    logger.warning(
                        "Failed to stop Capsem VM %s after cancelled start_vm", vid, exc_info=True
                    )
            raise
        _register_process_owned_vm(vid, task_name=task_name)
        return vid

    @classmethod
    async def _init_single_service(
        cls,
        controller: CapsemController,
        vm_id: str,
        cfg: CapsemSandboxConfig,
        *,
        use_oci_image: bool,
        task_name: str,
    ) -> dict[str, SandboxEnvironment]:
        container_id: str | None = None
        if cfg.execution_mode == "container":
            spec = cfg.to_container_spec()
            if use_oci_image:
                container_id = await prepare_oci_workload_container(controller, vm_id, spec)
            else:
                container_id = await start_container_for_init(controller, vm_id, spec)
        else:
            await controller.exec_in_vm(
                vm_id, f"mkdir -p {shlex.quote(cfg.working_dir)}", timeout=30
            )

        await bake_sandbox_tools_into_controller(
            controller,
            vm_id,
            container_id=container_id,
        )

        env = cls(
            vm_id=vm_id,
            controller=controller,
            container_id=container_id,
            working_dir=cfg.working_dir,
            execution_mode=cfg.execution_mode,
            owns_controller=True,
            task_name=task_name,
            user=cfg.user,
        )
        return {"default": env}

    @classmethod
    async def sample_init(
        cls,
        task_name: str,
        config: SandboxEnvironmentConfigType | str | None,
        metadata: dict[str, str],
    ) -> dict[str, SandboxEnvironment]:
        del metadata
        cfg = coerce_config(config)
        controller = SdkCapsemController()
        use_oci_image = _can_use_oci_image_create(cfg)
        vm_id = ""
        try:
            vm_id = await cls._start_vm_for_sample(
                controller, cfg, use_oci_image=use_oci_image, task_name=task_name
            )
            return await cls._init_single_service(
                controller,
                vm_id,
                cfg,
                use_oci_image=use_oci_image,
                task_name=task_name,
            )
        except BaseException:
            try:
                if vm_id:
                    await _teardown_failed_init(controller, vm_id)
            finally:
                with contextlib.suppress(Exception):
                    await controller.close()
            raise

    @classmethod
    async def sample_cleanup(
        cls,
        task_name: str,
        config: SandboxEnvironmentConfigType | str | None,
        environments: dict[str, SandboxEnvironment],
        interrupted: bool,
    ) -> None:
        del task_name, config, interrupted
        for env in environments.values():
            if isinstance(env, CapsemSandboxEnvironment):
                await env.cleanup()

    @classmethod
    async def cli_cleanup(cls, id: str | None) -> None:
        stopped_vms: set[str] = set()
        for env in list(cls._active_environments.values()):
            if id is None or id in (env.vm_id, env.container_id, env._instance_id):
                stopped_vms.add(env.vm_id)
                await env.cleanup()

        if id is None:
            for vid in await sweep_process_owned_vms():
                stopped_vms.add(vid)

        controller: CapsemController | None = None
        try:
            try:
                controller = SdkCapsemController()
                vms = await controller.list_vms()
            except Exception:
                logger.warning("Controller list_vms failed during cli_cleanup", exc_info=True)
                vms = []
            for vm_info in vms:
                vid = str(vm_info.get("id") or vm_info.get("name") or "")
                vname = str(vm_info.get("name") or vid)
                if not vid or vid in stopped_vms:
                    continue
                is_managed = vid.startswith(_MANAGED_VM_PREFIX) or vname.startswith(
                    _MANAGED_VM_PREFIX
                )
                if id is not None:
                    if id not in (vid, vname):
                        continue
                elif not is_managed:
                    continue
                try:
                    assert controller is not None
                    await controller.stop_vm(vid)
                    _unregister_process_owned_vm(vid)
                    stopped_vms.add(vid)
                except Exception:
                    logger.warning("Failed to clean up Capsem VM %s", vid, exc_info=True)
        finally:
            if controller is not None:
                with contextlib.suppress(Exception):
                    await controller.close()

    async def cleanup(self) -> None:
        """Stop the sandbox VM and close its controller."""
        CapsemSandboxEnvironment._active_environments.pop(self._instance_id, None)
        try:
            if not self._cleaned_up and self._vm_id:
                try:
                    await self._controller.stop_vm(self._vm_id)
                except Exception as exc:
                    logger.warning(
                        "Failed to stop Capsem VM %s (run 'inspect sandbox cleanup capsem' to retry): %s",
                        self._vm_id,
                        exc,
                    )
                else:
                    self._cleaned_up = True
                    _unregister_process_owned_vm(self._vm_id)
        finally:
            if self._owns_controller:
                self._owns_controller = False
                with contextlib.suppress(Exception):
                    await self._controller.close()

    def _in_container(self) -> bool:
        return self._execution_mode == "container" and bool(self._container_id)

    def _wrap_target_command(self, inner_script: str, *, as_root: bool = False) -> str:
        if self._in_container():
            script = f"{_CONTAINER_HOME_FIX}{inner_script}"
            if self._container_id == _OCI_WORKLOAD_CONTAINER_ID:
                runc_exec = _OCI_RUNC_EXEC_ROOT if as_root else _OCI_RUNC_EXEC
                return f"{runc_exec} bash -c {shlex.quote(script)}"
            user_flag = "-u 0 " if as_root else ""
            return (
                f"docker exec {user_flag}{shlex.quote(self._container_id or '')} "
                f"bash -c {shlex.quote(script)}"
            )
        return inner_script

    async def _run_raw_target(
        self, inner_script: str, *, timeout: int = 120, as_root: bool = False
    ) -> CommandResult:
        wrapped = self._wrap_target_command(inner_script, as_root=as_root)
        return await self._controller.exec_in_vm(self._vm_id, wrapped, timeout=timeout)

    async def _stage_bytes_to_guest(self, data: bytes, dest_guest_path: str) -> None:
        await _upload_via_transfer(
            self._controller,
            self._vm_id,
            self._container_id if self._execution_mode == "container" else None,
            dest_guest_path,
            data,
            container_user=self._user or "",
        )

    async def _prepare_exec_expr(
        self, quoted_cmd: str, input_data: str | bytes | None, temp_files: list[str]
    ) -> str:
        if len(quoted_cmd) > 65536:
            script_guest = f"/tmp/.capsem_cmd_{uuid.uuid4().hex[:10]}.sh"
            temp_files.append(script_guest)
            await self._stage_bytes_to_guest(quoted_cmd.encode("utf-8"), script_guest)
            exec_expr = f"bash {shlex.quote(script_guest)}"
        else:
            exec_expr = quoted_cmd

        if input_data is not None:
            input_bytes = input_data.encode("utf-8") if isinstance(input_data, str) else input_data
            if len(input_bytes) <= 16384:
                b64_in = base64.b64encode(input_bytes).decode("ascii")
                exec_expr = f"printf '%s' {shlex.quote(b64_in)} | base64 -d | {exec_expr}"
            else:
                stdin_guest = f"/tmp/.capsem_stdin_{uuid.uuid4().hex[:10]}.dat"
                temp_files.append(stdin_guest)
                await self._stage_bytes_to_guest(input_bytes, stdin_guest)
                exec_expr = f"{exec_expr} < {shlex.quote(stdin_guest)}"
        return exec_expr

    async def exec(
        self,
        cmd: list[str],
        input: str | bytes | None = None,
        cwd: str | None = None,
        env: dict[str, str] | None = None,
        user: str | None = None,
        timeout: int | None = None,
        timeout_retry: bool = True,
        concurrency: bool = True,
    ) -> ExecResult[str]:
        del timeout_retry, concurrency
        if not cmd:
            return ExecResult(success=True, returncode=0, stdout="", stderr="")

        effective_cwd = _resolve_guest_path(cwd, self._working_dir) if cwd else self._working_dir
        quoted_cmd = " ".join(shlex.quote(arg) for arg in cmd)

        temp_files_to_clean: list[str] = []
        cleaned_inline = False
        controller_timeout: int = EXEC_TIMEOUT_CEILING_SECS
        try:
            exec_expr = await self._prepare_exec_expr(quoted_cmd, input, temp_files_to_clean)
            effective_user = user or self._user
            run_as_root = bool(user) or not is_root_user_spec(effective_user)
            timed_script, controller_timeout = _format_exec_command(
                exec_expr,
                effective_cwd=effective_cwd,
                env=env,
                user=effective_user,
                timeout=timeout,
            )
            if temp_files_to_clean:
                rm_targets = " ".join(shlex.quote(p) for p in temp_files_to_clean)
                timed_script = f"( {timed_script} ); __ec=$?; rm -f {rm_targets}; exit $__ec"
            res = await self._run_raw_target(
                timed_script, timeout=controller_timeout, as_root=run_as_root
            )
            cleaned_inline = True
        except TimeoutError as exc:
            effective_timeout = timeout if timeout is not None else controller_timeout
            raise TimeoutError(f"Command timed out after {effective_timeout}s") from exc
        finally:
            if temp_files_to_clean and not cleaned_inline:
                rm_targets = " ".join(shlex.quote(p) for p in temp_files_to_clean)
                with contextlib.suppress(Exception):
                    await self._run_raw_target(f"rm -f {rm_targets}", timeout=15, as_root=True)

        stderr_str = res.stderr
        if timeout is not None and _TIMEOUT_SENTINEL in stderr_str:
            raise TimeoutError(f"Command timed out after {timeout}s")
        if res.exit_code == 126 and "permission denied" in stderr_str.lower():
            raise PermissionError(stderr_str.strip())

        limit_bytes = SandboxEnvironmentLimits.MAX_EXEC_OUTPUT_SIZE
        stdout_str, stdout_truncated = _truncate_utf8(res.stdout, limit_bytes)
        stderr_str, stderr_truncated = _truncate_utf8(stderr_str, limit_bytes)
        if stdout_truncated or stderr_truncated or res.truncated:
            raise OutputLimitExceededError(
                limit_str=SandboxEnvironmentLimits.MAX_EXEC_OUTPUT_SIZE_STR,
                truncated_output=stderr_str if stderr_truncated else stdout_str,
            )

        return ExecResult(
            success=(res.exit_code == 0),
            returncode=res.exit_code,
            stdout=stdout_str,
            stderr=stderr_str,
        )

    async def write_file(self, file: str, contents: str | bytes) -> None:
        resolved = _resolve_guest_path(file, self._working_dir)
        data = contents.encode("utf-8") if isinstance(contents, str) else contents

        stat_script = (
            f"if [ -d {shlex.quote(resolved)} ]; then echo 'IS_DIR'; exit 21; fi; "
            f"if [ -e {shlex.quote(resolved)} ]; then "
            f"  mode=$(stat -c '%a' {shlex.quote(resolved)} 2>/dev/null || echo 644); "
            f"  if [ $((0$mode & 0222)) -eq 0 ]; then echo 'NO_WRITE'; exit 13; fi; "
            f"fi"
        )
        stat_res = await self._run_raw_target(stat_script, timeout=30, as_root=True)
        if stat_res.exit_code == 21 or "IS_DIR" in stat_res.stdout:
            raise IsADirectoryError(f"Is a directory: '{file}'")
        if stat_res.exit_code == 13 or "NO_WRITE" in stat_res.stdout:
            raise PermissionError(f"Permission denied: '{file}'")

        await self._stage_bytes_to_guest(data, resolved)

    @staticmethod
    def _decode_text_bytes(raw_bytes: bytes, file: str) -> str:
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

    async def _fetch_guest_file_bytes(self, resolved: str) -> bytes:
        return await _download_via_transfer(
            self._controller,
            self._vm_id,
            self._container_id if self._execution_mode == "container" else None,
            resolved,
        )

    @overload
    async def read_file(self, file: str, text: Literal[True] = True) -> str: ...

    @overload
    async def read_file(self, file: str, text: Literal[False]) -> bytes: ...

    async def read_file(self, file: str, text: bool = True) -> str | bytes:
        resolved = _resolve_guest_path(file, self._working_dir)
        limit_bytes = SandboxEnvironmentLimits.MAX_READ_FILE_SIZE

        stat_script = (
            f"if [ ! -e {shlex.quote(resolved)} ]; then echo 'NOT_FOUND'; exit 2; fi; "
            f"if [ -d {shlex.quote(resolved)} ]; then echo 'IS_DIR'; exit 21; fi; "
            f"mode=$(stat -c '%a' {shlex.quote(resolved)} 2>/dev/null || echo 644); "
            f"if [ $((0$mode & 0444)) -eq 0 ]; then echo 'NO_READ'; exit 13; fi; "
            f"size=$(stat -c '%s' {shlex.quote(resolved)} 2>/dev/null || echo 0); "
            f'echo "SIZE:$size"'
        )
        stat_res = await self._run_raw_target(stat_script, timeout=30, as_root=True)
        if stat_res.exit_code == 2 or "NOT_FOUND" in stat_res.stdout:
            raise FileNotFoundError(f"No such file or directory: '{file}'")
        if stat_res.exit_code == 21 or "IS_DIR" in stat_res.stdout:
            raise IsADirectoryError(f"Is a directory: '{file}'")
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

        raw_bytes = await self._fetch_guest_file_bytes(resolved)
        if len(raw_bytes) > limit_bytes:
            raise OutputLimitExceededError(
                limit_str=SandboxEnvironmentLimits.MAX_READ_FILE_SIZE_STR, truncated_output=None
            )
        if not text:
            return raw_bytes
        return self._decode_text_bytes(raw_bytes, file)

    async def connection(self, *, user: str | None = None) -> SandboxConnection:
        user_flag = f"-u {shlex.quote(user)} " if user else ""
        if self._execution_mode == "container" and self._container_id:
            if self._container_id == _OCI_WORKLOAD_CONTAINER_ID:
                runc_cmd = (
                    f"{_OCI_RUNC_EXEC_PREFIX} -u {shlex.quote(user)} workload"
                    if user
                    else _OCI_RUNC_EXEC
                )
                command = f"capsem exec {shlex.quote(self._vm_id)} -- {runc_cmd} bash"
            else:
                command = (
                    f"capsem exec {shlex.quote(self._vm_id)} -- "
                    f"docker exec -it {user_flag}{shlex.quote(self._container_id)} bash"
                )
        else:
            command = f"capsem exec {shlex.quote(self._vm_id)} -- bash"
        return SandboxConnection(
            type="capsem", command=command, container=self._container_id or self._vm_id
        )


__all__ = [
    "INSPECT_SANDBOX_TOOLS_GUEST_DIR",
    "INSPECT_SANDBOX_TOOLS_GUEST_PATH",
    "CapsemController",
    "CapsemSandboxConfig",
    "CapsemSandboxEnvironment",
    "CommandResult",
    "SdkCapsemController",
    "bake_sandbox_tools_into_controller",
    "resolve_inspect_sandbox_tools_host_binary",
]
