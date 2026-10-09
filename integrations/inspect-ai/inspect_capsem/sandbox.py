"""Standalone Capsem SandboxEnvironment for Inspect AI."""

from __future__ import annotations

import asyncio
import atexit
import contextlib
import logging
import shlex
import uuid
from typing import Any, ClassVar, Literal, overload

from inspect_ai.util import ExecResult, SandboxConnection, SandboxEnvironment
from inspect_ai.util import SandboxEnvironmentConfigType as _CfgType

from inspect_capsem import _cleanup as _cl
from inspect_capsem import _lifecycle as _lc
from inspect_capsem._compose import coerce_config
from inspect_capsem._controller import CapsemController, SdkCapsemController
from inspect_capsem._exec import exec_in_sandbox
from inspect_capsem._files import read_guest_file, write_guest_file
from inspect_capsem._lifecycle import sweep_leftover_vms
from inspect_capsem._tools import bake_sandbox_tools_into_controller
from inspect_capsem.config import CapsemSandboxConfig

logger = logging.getLogger(__name__)


async def sweep_process_owned_vms(
    task_name: str | None = None, controller: CapsemController | None = None
) -> list[str]:
    """Stop any VMs started by this process (or by `task_name`) that have not yet been stopped."""
    return await _lc.sweep_process_owned_vms(
        task_name=task_name, controller=controller, controller_factory=lambda: SdkCapsemController()
    )


@atexit.register
def _atexit_sweep_process_owned_vms() -> None:
    with contextlib.suppress(Exception):
        coro = sweep_process_owned_vms()
        asyncio.run(asyncio.wait_for(coro, timeout=_lc._INIT_TEARDOWN_TIMEOUT_SECS))


class CapsemSandboxEnvironment(SandboxEnvironment):
    """Spec-compliant Inspect AI `SandboxEnvironment` backed by Capsem."""

    _active_environments: ClassVar[dict[str, CapsemSandboxEnvironment]] = {}

    def __init__(
        self,
        vm_id: str,
        controller: CapsemController | None = None,
        *,
        working_dir: str = "/workspace",
        execution_mode: Literal["vm", "container"] = "vm",
        owns_controller: bool | None = None,
        task_name: str | None = None,
        user: str | None = None,
    ) -> None:
        super().__init__()
        self._vm_id = vm_id
        self._working_dir = working_dir
        self._execution_mode: Literal["vm", "container"] = execution_mode
        self._cleaned_up = False
        self._controller = SdkCapsemController() if controller is None else controller
        self._owns_controller = (controller is None) if owns_controller is None else owns_controller
        self._task_name = task_name
        self._user = user
        self._instance_id = f"{vm_id}:{execution_mode}:{uuid.uuid4().hex[:6]}"
        CapsemSandboxEnvironment._active_environments[self._instance_id] = self

    @property
    def vm_id(self) -> str:
        return self._vm_id

    @property
    def execution_mode(self) -> Literal["vm", "container"]:
        return self._execution_mode

    @classmethod
    def config_files(cls) -> list[str]:
        compose = [f"{p}.{e}" for p in ("compose", "docker-compose") for e in ("yaml", "yml")]
        return [*compose, "Dockerfile", "Containerfile"]

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
    async def task_init(cls, task_name: str, config: _CfgType | str | None) -> None:
        del task_name, config
        with contextlib.suppress(Exception):
            controller = SdkCapsemController()
            try:
                coro = sweep_leftover_vms(controller)
                await asyncio.wait_for(coro, timeout=_lc._INIT_TEARDOWN_TIMEOUT_SECS)
            finally:
                await controller.close()

    @classmethod
    async def task_cleanup(
        cls, task_name: str, config: _CfgType | str | None, cleanup: bool
    ) -> None:
        del config
        envs = [e for e in cls._active_environments.values() if task_name in (None, e._task_name)]
        if not cleanup:
            surviving_ids = _lc._abandon_process_owned_vms(task_name)
            await _cl._abandon_task_environments(cls._active_environments, envs, surviving_ids)
            return

        for env in envs:
            with contextlib.suppress(Exception):
                await env.cleanup()
        with contextlib.suppress(Exception):
            coro = sweep_process_owned_vms(task_name)
            await asyncio.wait_for(coro, timeout=_lc._INIT_TEARDOWN_TIMEOUT_SECS)

    @classmethod
    async def sample_init(
        cls, task_name: str, config: _CfgType | str | None, metadata: dict[str, str]
    ) -> dict[str, SandboxEnvironment]:
        cfg = coerce_config(config, sample_metadata=metadata)
        controller = SdkCapsemController()
        vm_id = ""
        try:
            vm_id = await _lc._init_sample_vm(controller, cfg, task_name)
            working_dir = cfg.working_dir or "/workspace"
            if cfg.execution_mode == "container":
                from inspect_capsem import containers as _c

                spec = cfg.to_container_spec()
                await _c.prepare_oci_workload_container(controller, vm_id, spec)
                if not spec.working_dir_explicit:
                    working_dir = await _c.resolve_container_working_dir(controller, vm_id)
            else:
                mkdir_cmd = f"mkdir -p {shlex.quote(working_dir)}"
                await controller.exec_in_vm(vm_id, mkdir_cmd, timeout=30)
            await bake_sandbox_tools_into_controller(controller, vm_id)
            env = cls(
                vm_id,
                controller,
                working_dir=working_dir,
                execution_mode=cfg.execution_mode,
                task_name=task_name,
                user=cfg.user,
            )
            env._owns_controller = True
            return {"default": env}
        except BaseException:
            try:
                if vm_id:
                    await _lc._teardown_failed_init(
                        controller, vm_id, _lc._INIT_TEARDOWN_TIMEOUT_SECS
                    )
            finally:
                with contextlib.suppress(Exception):
                    await controller.close()
            raise

    @classmethod
    async def sample_cleanup(
        cls,
        task_name: str,
        config: _CfgType | str | None,
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
            if id is None or id in (env.vm_id, env._instance_id):
                stopped_vms.add(env.vm_id)
                await env.cleanup()
        if id is None:
            stopped_vms.update(await sweep_process_owned_vms())
        await _cl._sweep_untracked_managed_vms(
            lambda: SdkCapsemController(), id, stopped_vms, _lc._unregister_process_owned_vm
        )

    async def cleanup(self) -> None:
        await _cl._cleanup_environment(self, _lc._unregister_process_owned_vm)

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
        args = (self._working_dir, self._user, input, cwd, env, user, timeout)
        return await exec_in_sandbox(
            self._controller,
            self._vm_id,
            cmd,
            *args,
            is_container=self._execution_mode == "container",
        )

    async def write_file(self, file: str, contents: str | bytes) -> None:
        await write_guest_file(
            self._controller,
            self._vm_id,
            self._working_dir,
            file,
            contents,
            is_container=self._execution_mode == "container",
            default_user=self._user,
        )

    @overload
    async def read_file(self, file: str, text: Literal[True] = True) -> str: ...
    @overload
    async def read_file(self, file: str, text: Literal[False]) -> bytes: ...

    async def read_file(self, file: str, text: bool = True) -> str | bytes:
        return await read_guest_file(
            self._controller, self._vm_id, self._working_dir, file, text=text
        )

    async def connection(self, *, user: str | None = None) -> SandboxConnection:
        del user
        return SandboxConnection(
            type="capsem",
            command=f"capsem shell {shlex.quote(self._vm_id)}",
        )


__all__ = ["CapsemSandboxEnvironment"]
