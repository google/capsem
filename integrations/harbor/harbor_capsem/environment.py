"""Harbor `BaseEnvironment` implementation backed by Capsem micro-VMs."""

from __future__ import annotations

import asyncio
import contextlib
import math
import os
import shlex
from collections.abc import Sequence
from pathlib import Path
from typing import Any, Literal
from uuid import UUID

from capsem.execution import EXEC_TIMEOUT_CEILING_SECS
from capsem_integrations import _controller, _exec, containers
from capsem_integrations.containers.image_build import build_and_stage_oci_image
from harbor.environments.base import BaseEnvironment, ExecResult
from harbor.environments.capabilities import (
    EnvironmentCapabilities,
    EnvironmentResourceCapabilities,
)
from harbor.environments.definition import effective_exec_cwd
from harbor.models.task.config import EnvironmentConfig
from harbor.models.trial.paths import TrialPaths

from harbor_capsem import _trace
from harbor_capsem._credentials import (
    _RUNTIME_CA_ENV,
    broker_env_credentials,
    coerce_bool_flag,
    truncate_label_value,
)
from harbor_capsem._transfer import (
    _INIT_TEARDOWN_TIMEOUT_SECS,
    _harbor_build_policy,
    _resolve_container_start_inputs,
    _start_vm_shielded,
    _validate_environment_definition,
    download_dir,
    download_file,
    upload_dir,
    upload_file,
)


class CapsemEnvironment(BaseEnvironment):
    """Harbor `BaseEnvironment` running tasks inside Capsem micro-VMs."""

    def __init__(
        self,
        environment_dir: Path,
        environment_name: str,
        session_id: str,
        trial_paths: TrialPaths,
        task_env_config: EnvironmentConfig,
        *args: Any,
        execution_mode: Literal["container", "vm"] = "container",
        broker_credentials: bool | str = True,
        export_trace: bool | str | None = None,
        include_security_events: bool | str | None = None,
        strict_trace_export: bool | str = False,
        controller: _controller.CapsemController | None = None,
        host_build: Any = None,
        allowed_host_paths: Sequence[str] = (),
        allowed_host_env: Sequence[str] = (),
        **kwargs: Any,
    ) -> None:
        if execution_mode not in ("container", "vm"):
            raise ValueError(f"Invalid execution_mode {execution_mode!r}; expected container|vm.")
        self._execution_mode: Literal["container", "vm"] = execution_mode
        self._broker_credentials = coerce_bool_flag(broker_credentials, default=True)
        self._export_trace, self._include_security_events = _trace.resolve_trace_options(
            export_trace=export_trace, include_security_events=include_security_events
        )
        self._strict_trace_export = coerce_bool_flag(strict_trace_export, default=False)
        self._credential_cache: dict[tuple[str, str], str] = {}
        self._controller = _controller.SdkCapsemController() if controller is None else controller
        self._owns_controller = controller is None
        self._host_build = host_build
        self._allowed_host_paths = tuple(str(p) for p in allowed_host_paths)
        self._allowed_host_env = tuple(str(e) for e in allowed_host_env)
        self._compose_file: Path | None = None
        self._compose_fields: dict[str, Any] = {}
        self._vm_id = ""
        self._dockerfile_workdir: str | None = None
        self.context_id: UUID | None = kwargs.pop("context_id", None)
        base_args = (environment_dir, environment_name, session_id, trial_paths, task_env_config)
        super().__init__(*base_args, *args, **kwargs)

    @property
    def vm_id(self) -> str:
        return self._vm_id

    @property
    def execution_mode(self) -> Literal["container", "vm"]:
        return self._execution_mode

    @staticmethod
    def type() -> str:
        return "capsem"

    @property
    def capabilities(self) -> EnvironmentCapabilities:
        return EnvironmentCapabilities(
            disable_internet=True, gpus=False, docker_compose=False, mounted=False
        )

    @classmethod
    def resource_capabilities(cls) -> EnvironmentResourceCapabilities:
        return EnvironmentResourceCapabilities(
            cpu_limit=True, cpu_request=True, memory_limit=True, memory_request=True
        )

    def _validate_definition(self) -> None:
        self._compose_file, self._compose_fields = _validate_environment_definition(
            self.environment_dir,
            execution_mode=self._execution_mode,
            docker_image=self.task_env_config.docker_image,
            allowed_host_paths=self._allowed_host_paths,
            allowed_host_env=self._allowed_host_env,
            host_build=self._host_build,
            allow_internet=self.task_env_config.allow_internet is not False,
        )

    async def start(self, force_build: bool) -> None:
        cpus = max(1, int(self._effective_cpus or self._compose_fields.get("cpu_count") or 4))
        eff_mem, cfg_ram = self._effective_memory_mb, int(self._compose_fields.get("ram_gb") or 8)
        ram_gb = max(1, math.ceil(eff_mem / 1024) if eff_mem else cfg_ram)
        allow_net = self.task_env_config.allow_internet is not False
        vm_id = ""
        try:
            oci_image, build_spec, oci_cmd = _resolve_container_start_inputs(
                self.environment_dir,
                execution_mode=self._execution_mode,
                docker_image=self.task_env_config.docker_image,
                force_build=force_build,
                compose_fields=self._compose_fields,
                allowed_host_paths=self._allowed_host_paths,
                host_build=self._host_build,
                allow_internet=allow_net,
            )
            ca_pem: str | None = None
            if self._execution_mode == "container":
                if build_spec is not None:
                    b_paths = (*self._allowed_host_paths, str(build_spec.get("context") or ""))
                    with _harbor_build_policy(
                        self.environment_dir, b_paths, self._host_build, allow_internet=allow_net
                    ):
                        oci_image, ca_pem = await build_and_stage_oci_image(build_spec)
                if not oci_image:
                    raise ValueError("Container mode requires docker_image, Dockerfile, or compose.")
            c_env = dict(self._compose_fields.get("environment") or {})
            startup_env = await self._broker_env({**c_env, **self._startup_env()})
            labels = {
                "harbor-session-id": truncate_label_value(str(self.session_id)),
                "harbor-env": truncate_label_value(str(self.environment_name)),
            }
            if prefix := os.environ.get("CAPSEM_VM_PREFIX", "").strip():
                labels["capsem-vm-prefix"] = truncate_label_value(prefix)
            extra_kw: dict[str, Any] = {"command": oci_cmd, "env": startup_env, "labels": labels}
            if ca_pem:
                extra_kw["registry_ca_pem"] = ca_pem
            self._vm_id = vm_id = await _start_vm_shielded(
                self._controller, cpu_count=cpus, ram_gb=ram_gb, image=oci_image, **extra_kw
            )
            if self._execution_mode == "container":
                cfg_wd = self.task_env_config.workdir or self._compose_fields.get("working_dir")
                host_paths = (str(self.environment_dir.resolve()), *self._allowed_host_paths)
                spec = containers.ContainerSpec(
                    image=oci_image,
                    working_dir=str(cfg_wd or "/workspace"),
                    working_dir_explicit=cfg_wd is not None,
                    environment=startup_env,
                    command=oci_cmd,
                    volumes=tuple(self._compose_fields.get("volumes") or ()),
                    healthcheck=self._compose_fields.get("healthcheck"),
                    user=self._compose_fields.get("user"),
                    allowed_host_paths=host_paths,
                    build=build_spec,
                )
                await containers.prepare_oci_workload_container(self._controller, vm_id, spec)
                resolved_wd = (
                    str(cfg_wd)
                    if cfg_wd
                    else await containers.resolve_container_working_dir(self._controller, vm_id)
                )
                self._dockerfile_workdir = resolved_wd
            else:
                self._dockerfile_workdir = self.task_env_config.workdir or "/workspace"
                mkdir_cmd = f"mkdir -p {shlex.quote(self._dockerfile_workdir)}"
                await self._controller.exec_in_vm(vm_id, mkdir_cmd, timeout=30)
            await self.ensure_dirs(self._mount_targets(writable_only=True))
            await self._upload_environment_dir_after_start()
        except BaseException:
            self._vm_id = ""
            try:
                if vm_id:
                    with contextlib.suppress(Exception):
                        await asyncio.wait_for(
                            asyncio.shield(self._controller.stop_vm(vm_id)),
                            timeout=_INIT_TEARDOWN_TIMEOUT_SECS,
                        )
            finally:
                with contextlib.suppress(Exception):
                    await self._controller.close()
            raise

    async def stop(self, delete: bool) -> None:
        vm_id = self._vm_id
        try:
            if not delete:
                raise NotImplementedError(
                    "Capsem environments use lease-owned managed sessions (Hypervisor.managed()) "
                    "and never outlive the trial process; stop(delete=False) is not supported."
                )
            if vm_id and self._export_trace:
                await _trace.stop_and_export_trial_trace(
                    self._controller,
                    vm_id,
                    trial_paths=self.trial_paths,
                    session_id=self.session_id,
                    include_security_events=self._include_security_events,
                    strict=self._strict_trace_export,
                )
        finally:
            try:
                if vm_id:
                    self._vm_id = ""
                    await self._controller.stop_vm(vm_id)
            finally:
                if self._owns_controller:
                    with contextlib.suppress(Exception):
                        await self._controller.close()

    def _require_vm(self) -> str:
        if not self._vm_id:
            raise RuntimeError("CapsemEnvironment is not started; call start() first.")
        return self._vm_id

    def _default_working_dir(self) -> str:
        fb = "/workspace" if self._execution_mode == "vm" else "/"
        wd = self.task_env_config.workdir
        return effective_exec_cwd(None, wd, self._dockerfile_workdir) or fb

    async def _broker_env(self, raw_env: dict[str, str]) -> dict[str, str]:
        return await broker_env_credentials(
            self._controller,
            {**_RUNTIME_CA_ENV, **raw_env},
            enabled=self._broker_credentials,
            cache=self._credential_cache,
        )

    async def exec(
        self,
        command: str,
        cwd: str | None = None,
        env: dict[str, str] | None = None,
        timeout_sec: int | None = None,
        user: str | int | None = None,
    ) -> ExecResult:
        vm_id, is_cont = self._require_vm(), self._execution_mode == "container"
        merged_env = await self._broker_env(self._merge_env(env) or {})
        raw_user = self._resolve_user(user) or self._compose_fields.get("user")
        eff_user = str(raw_user) if raw_user is not None else None
        eff_cwd = self._default_working_dir() if cwd is None else cwd
        ceiling = EXEC_TIMEOUT_CEILING_SECS
        clamped = min(max(1, int(timeout_sec)), ceiling) if timeout_sec is not None else None
        q_cmd = shlex.quote(command)
        shell_expr = (
            f"if command -v bash >/dev/null 2>&1; then exec bash -c {q_cmd}; "
            f"else exec sh -c {q_cmd}; fi"
        )
        timed_script, ctrl_timeout = _exec._format_exec_command(
            shell_expr, effective_cwd=eff_cwd, env=merged_env, user=eff_user, timeout=clamped
        )
        wrapped = _exec._wrap_target_command(timed_script, is_container=is_cont, user=eff_user)
        res = await self._controller.exec_in_vm(vm_id, wrapped, timeout=min(ctrl_timeout, ceiling))
        if cb := self._output_callback():
            if res.stdout:
                await cb(res.stdout, "stdout")
            if res.stderr:
                await cb(res.stderr, "stderr")
        return ExecResult(stdout=res.stdout, stderr=res.stderr, return_code=res.exit_code)

    async def upload_file(self, source_path: Path | str, target_path: str) -> None:
        wd, vm = self._default_working_dir(), self._require_vm()
        await upload_file(self._controller, vm, source_path, target_path, working_dir=wd)

    async def download_file(self, source_path: str, target_path: Path | str) -> None:
        wd, vm = self._default_working_dir(), self._require_vm()
        await download_file(self._controller, vm, source_path, target_path, working_dir=wd)

    async def upload_dir(self, source_dir: Path | str, target_dir: str) -> None:
        wd, vm = self._default_working_dir(), self._require_vm()
        await upload_dir(self._controller, vm, source_dir, target_dir, working_dir=wd)

    async def download_dir(self, source_dir: str, target_dir: Path | str) -> None:
        wd, vm = self._default_working_dir(), self._require_vm()
        await download_dir(self._controller, vm, source_dir, target_dir, working_dir=wd)
