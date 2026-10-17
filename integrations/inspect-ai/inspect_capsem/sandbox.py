"""Standalone Capsem SandboxEnvironment for Inspect AI."""

from __future__ import annotations

import asyncio
import contextlib
import logging
import os
import shlex
import uuid
from typing import Any, ClassVar, Literal, overload

from inspect_ai.util import ExecResult, SandboxConnection, SandboxEnvironment
from inspect_ai.util import SandboxEnvironmentConfigType as _CfgType

from inspect_capsem._compose import coerce_config
from inspect_capsem._controller import (
    CapsemController,
    SdkCapsemController,
    truncate_label_value,
)
from inspect_capsem._exec import exec_in_sandbox
from inspect_capsem._files import read_guest_file, write_guest_file
from inspect_capsem._tools import bake_sandbox_tools_into_controller
from inspect_capsem.config import CapsemSandboxConfig

logger = logging.getLogger(__name__)

_INIT_TEARDOWN_TIMEOUT_SECS = 25.0


def _oci_create_command(cfg: CapsemSandboxConfig) -> tuple[str, ...] | None:
    if cfg.command is None:
        return None
    if isinstance(cfg.command, str):
        return tuple(shlex.split(cfg.command)) if cfg.command.strip() else None
    return tuple(cfg.command)


def _sample_vm_labels(task_name: str | None = None) -> dict[str, str] | None:
    labels = {"inspect-capsem-task": truncate_label_value(task_name)} if task_name else {}
    if prefix := os.environ.get("CAPSEM_VM_PREFIX", "").strip():
        labels["capsem-vm-prefix"] = truncate_label_value(prefix)
    return labels or None


async def _start_sample_vm(
    controller: CapsemController, cfg: CapsemSandboxConfig, task_name: str | None = None
) -> str:
    """Start a managed VM for `sample_init`, tearing down on cancellation."""
    oci_image = cfg.image if cfg.execution_mode == "container" else None
    ca_pem: str | None = None
    if cfg.execution_mode == "container" and cfg.build:
        from inspect_capsem.containers.image_build import build_and_stage_oci_image

        oci_image, ca_pem = await build_and_stage_oci_image(cfg.build)
    oci_cmd = _oci_create_command(cfg) if cfg.execution_mode == "container" else None
    extra_kw: dict[str, Any] = {"registry_ca_pem": ca_pem} if ca_pem else {}
    if labels := _sample_vm_labels(task_name):
        extra_kw["labels"] = labels
    env_dict = dict(cfg.environment) if cfg.environment else None
    start_task = asyncio.ensure_future(
        controller.start_vm(
            cpu_count=cfg.cpu_count,
            ram_gb=cfg.ram_gb,
            image=oci_image,
            command=oci_cmd,
            env=env_dict,
            **extra_kw,
        )
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
            try:
                await controller.stop_vm(vm_id)
            except Exception as exc:
                logger.warning("Failed to stop Capsem VM %s after cancelled start_vm: %s", vm_id, exc)
        else:
            start_task.cancel()
            with contextlib.suppress(BaseException):
                await start_task
        raise


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

    @classmethod
    async def task_cleanup(
        cls, task_name: str, config: _CfgType | str | None, cleanup: bool
    ) -> None:
        del config
        envs = [
            e for e in list(cls._active_environments.values()) if task_name in (None, e._task_name)
        ]
        for env in envs:
            with contextlib.suppress(Exception):
                await env.cleanup()
            cls._active_environments.pop(env._instance_id, None)
            if env._owns_controller:
                env._owns_controller = False
                with contextlib.suppress(Exception):
                    await env._controller.close()
        if not cleanup:
            raise NotImplementedError(
                "Capsem sandboxes use lease-owned managed sessions (Hypervisor.managed()) and "
                "never outlive the evaluation process; cleanup=False (--no-sandbox-cleanup) "
                "is not supported."
            )

    @classmethod
    async def sample_init(
        cls, task_name: str, config: _CfgType | str | None, metadata: dict[str, str]
    ) -> dict[str, SandboxEnvironment]:
        cfg = coerce_config(config, sample_metadata=metadata)
        controller = SdkCapsemController()
        vm_id = ""
        try:
            vm_id = await _start_sample_vm(controller, cfg, task_name=task_name)
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
                owns_controller=True,
                task_name=task_name,
                user=cfg.user,
            )
            return {"default": env}
        except BaseException:
            try:
                if vm_id:
                    try:
                        await asyncio.wait_for(
                            asyncio.shield(controller.stop_vm(vm_id)),
                            timeout=_INIT_TEARDOWN_TIMEOUT_SECS,
                        )
                    except Exception as exc:
                        logger.warning(
                            "Cleanup after failed sample_init of VM %s failed: %s", vm_id, exc
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

    async def cleanup(self) -> None:
        if not self._cleaned_up and self._vm_id:
            try:
                await self._controller.stop_vm(self._vm_id)
                self._cleaned_up = True
            except Exception as exc:
                logger.warning("Failed to stop Capsem VM %s: %s", self._vm_id, exc)
                return
        CapsemSandboxEnvironment._active_environments.pop(self._instance_id, None)
        if self._owns_controller:
            self._owns_controller = False
            with contextlib.suppress(Exception):
                await self._controller.close()

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
        args = (self._controller, self._vm_id, self._working_dir, file, contents)
        await write_guest_file(
            *args, is_container=self._execution_mode == "container", default_user=self._user
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
        return SandboxConnection(type="capsem", command=f"capsem shell {shlex.quote(self._vm_id)}")


__all__ = ["CapsemSandboxEnvironment"]
