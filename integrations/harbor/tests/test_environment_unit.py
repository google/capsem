"""Hermetic unit tests for `CapsemEnvironment` and `harbor_capsem._transfer`."""

from __future__ import annotations

import ast
import asyncio
import os
import re
import subprocess
from collections.abc import Mapping
from pathlib import Path
from typing import Any

import harbor_capsem
import pytest
from capsem_integrations._controller import CommandResult
from capsem_integrations.containers import image_build as _ib
from harbor.models.task.config import EnvironmentConfig
from harbor.models.trial.paths import TrialPaths
from harbor_capsem import CapsemEnvironment
from harbor_capsem import _transfer as _xfer


class FakeCapsemController:
    """Host-backed fake controller executing guest commands in a local temp directory."""

    def __init__(self, root_dir: Path) -> None:
        self.root_dir = root_dir
        self.started_vms: list[dict[str, Any]] = []
        self.stopped_vms: list[str] = []
        self.commands: list[tuple[str, int]] = []
        self.uploads: dict[str, bytes] = {}
        self.files: dict[str, bytes] = {}
        self.closed = False

    async def start_vm(
        self, *, cpu_count: int, ram_gb: int, image: str | None = None, **kwargs: Any
    ) -> str:
        vm_id = f"vm-harbor-{len(self.started_vms)}"
        cmd, env, lbls = kwargs.get("command"), kwargs.get("env"), kwargs.get("labels")
        info: dict[str, Any] = {"vm_id": vm_id, "cpu_count": cpu_count, "ram_gb": ram_gb}
        info.update({"image": image, "command": tuple(cmd) if cmd else None})
        info["env"] = dict(env) if env else None
        info["labels"] = dict(lbls) if lbls else {}
        info["registry_ca_pem"] = kwargs.get("registry_ca_pem")
        self.started_vms.append(info)
        return vm_id

    async def stop_vm(self, vm_id: str) -> None:
        self.stopped_vms.append(vm_id)

    async def close(self) -> None:
        self.closed = True

    def _host_path(self, guest_path: str) -> Path:
        return self.root_dir / guest_path.lstrip("/")

    async def upload_to_vm(self, vm_id: str, guest_path: str, data: bytes) -> None:
        del vm_id
        self.uploads[guest_path] = self.files[guest_path] = data
        target = self._host_path(guest_path)
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)

    async def download_from_vm(
        self, vm_id: str, guest_path: str, *, max_bytes: int | None = None
    ) -> bytes:
        del vm_id
        data = self.files.get(guest_path) or self._host_path(guest_path).read_bytes()
        return data[:max_bytes] if max_bytes is not None else data

    async def exec_in_vm(self, vm_id: str, command: str, *, timeout: int = 120) -> CommandResult:
        del vm_id
        self.commands.append((command, timeout))
        if command == "pwd":
            return CommandResult(exit_code=0, stdout="/workspace\n", stderr="")
        if ".hb-upload-" in command or ".hb-download-" in command:
            rewritten = re.sub(
                r"/(var/tmp|workspace)(?=/|\s|'|$)",
                lambda m: f"{self.root_dir}/{m.group(1)}",
                command,
            )
            (self.root_dir / "var/tmp").mkdir(parents=True, exist_ok=True)
            proc = subprocess.run(
                ["bash", "-c", rewritten], capture_output=True, text=True, check=False
            )
            return CommandResult(exit_code=proc.returncode, stdout=proc.stdout, stderr=proc.stderr)
        if command.startswith("mkdir -p "):
            return CommandResult(exit_code=0, stdout="", stderr="")
        return CommandResult(exit_code=0, stdout="exec-ok\n", stderr="")


def _make_trial_paths(tmp_path: Path) -> TrialPaths:
    trial_dir = tmp_path / "trial"
    trial_dir.mkdir(parents=True, exist_ok=True)
    return TrialPaths(trial_dir=trial_dir)


def test_capabilities_resource_contract_and_no_inspect_capsem_imports(tmp_path: Path) -> None:
    pkg_dir = Path(harbor_capsem.__file__).resolve().parent
    for py_file in pkg_dir.rglob("*.py"):
        tree = ast.parse(py_file.read_text(encoding="utf-8"), filename=str(py_file))
        for node in ast.walk(tree):
            if isinstance(node, ast.Import):
                assert all(not a.name.startswith("inspect_capsem") for a in node.names)
            elif isinstance(node, ast.ImportFrom) and node.module:
                assert not node.module.startswith("inspect_capsem")

    env_dir, tp = tmp_path / "env", _make_trial_paths(tmp_path)
    env_dir.mkdir()
    ctrl = FakeCapsemController(tmp_path / "guest")
    tcfg = EnvironmentConfig(docker_image="ubuntu:24.04")
    env = CapsemEnvironment(
        env_dir, "hello-world", "hello-world__trial1__env", tp, tcfg, controller=ctrl
    )
    assert env.type() == "capsem" and env.execution_mode == "container"
    caps = env.capabilities
    assert caps.disable_internet is True and not (caps.gpus or caps.docker_compose or caps.mounted)
    rcaps = CapsemEnvironment.resource_capabilities()
    assert rcaps.cpu_limit and rcaps.cpu_request and rcaps.memory_limit and rcaps.memory_request
    assert _xfer._resolve_guest_path("rel/a.txt", "/workspace") == "/workspace/rel/a.txt"


async def test_container_and_vm_modes_lifecycle_and_exec_clamping(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("CAPSEM_VM_PREFIX", "hb-unit")
    env_dir, tp = tmp_path / "env", _make_trial_paths(tmp_path)
    env_dir.mkdir()
    ctrl = FakeCapsemController(tmp_path / "guest")
    ek: dict[str, Any] = {"cpus": 2, "memory_mb": 4096, "workdir": "/app"}
    tcfg = EnvironmentConfig(
        docker_image="debian:bookworm-slim", env={"TASK_VAR": "from-task"}, **ek
    )
    env = CapsemEnvironment(env_dir, "task-a", "task-a__1__env", tp, tcfg, controller=ctrl)
    await env.start(force_build=False)
    assert env.vm_id == "vm-harbor-0"
    started = ctrl.started_vms[0]
    assert (started["cpu_count"], started["ram_gb"]) == (2, 4)
    assert started["image"] == "debian:bookworm-slim"
    assert started["labels"]["harbor-session-id"] == "task-a__1__env"
    assert started["labels"]["harbor-env"] == "task-a"
    assert started["labels"]["capsem-vm-prefix"] == "hb-unit"
    assert started["env"]["TASK_VAR"] == "from-task"
    assert started["env"]["SSL_CERT_FILE"] == "/etc/ssl/certs/ca-certificates.crt"

    res = await env.exec("echo hi", env={"EXTRA": "1"}, timeout_sec=9999, user="root")
    assert res.return_code == 0 and res.stdout == "exec-ok\n"
    last_cmd, last_timeout = ctrl.commands[-1]
    assert last_timeout == 3600 and "cd /app" in last_cmd and "EXTRA=1" in last_cmd
    assert "SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt" in last_cmd

    with pytest.raises(NotImplementedError, match=r"stop\(delete=False\) is not supported"):
        await env.stop(delete=False)
    assert ctrl.stopped_vms == ["vm-harbor-0"] and env.vm_id == ""

    vm_env_dir = tmp_path / "vm_env"
    vm_env_dir.mkdir()
    vm_env = CapsemEnvironment(
        vm_env_dir,
        "vm-task",
        "vm-task__1__env",
        tp,
        EnvironmentConfig(),
        execution_mode="vm",
        controller=ctrl,
    )
    await vm_env.start(force_build=False)
    assert ctrl.started_vms[1]["image"] is None
    await vm_env.stop(delete=True)
    assert ctrl.stopped_vms == ["vm-harbor-0", "vm-harbor-1"]


async def test_single_service_compose_and_multi_service_rejection(tmp_path: Path) -> None:
    single_dir, tp = tmp_path / "single", _make_trial_paths(tmp_path)
    single_dir.mkdir()
    (single_dir / "docker-compose.yaml").write_text(
        "services:\n  main:\n    image: python:3.12-slim\n    working_dir: /work\n"
        "    user: '1000:1000'\n    environment:\n      COMPOSE_FOO: bar\n"
        "    command: ['sleep', 'infinity']\n",
        encoding="utf-8",
    )
    ctrl = FakeCapsemController(tmp_path / "guest")
    env = CapsemEnvironment(single_dir, "c-ok", "c-ok__1", tp, EnvironmentConfig(), controller=ctrl)
    await env.start(force_build=False)
    started = ctrl.started_vms[0]
    assert started["image"] == "python:3.12-slim" and started["command"] == ("sleep", "infinity")
    assert started["env"]["COMPOSE_FOO"] == "bar"
    await env.exec("id")
    assert "1000:1000" in ctrl.commands[-1][0]
    await env.stop(delete=True)

    multi_dir = tmp_path / "multi"
    multi_dir.mkdir()
    (multi_dir / "docker-compose.yaml").write_text(
        "services:\n  main:\n    image: alpine:3.19\n  db:\n    image: redis:7\n",
        encoding="utf-8",
    )
    with pytest.raises(ValueError, match="Multi-service Compose files are not supported"):
        CapsemEnvironment(multi_dir, "c-m", "c-m__1", tp, EnvironmentConfig(), controller=ctrl)


async def test_dockerfile_build_and_file_dir_tar_transfers(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    df_dir, tp = tmp_path / "df_task", _make_trial_paths(tmp_path)
    df_dir.mkdir()
    df_text = "FROM busybox:latest\nWORKDIR /workspace\n"
    (df_dir / "Dockerfile").write_text(df_text, encoding="utf-8")

    async def _fake_build(spec: Mapping[str, Any]) -> tuple[str, str | None]:
        assert Path(spec["dockerfile"]).name == "Dockerfile"
        return "127.0.0.1:5055/capsem-inspect-build:test", "-----BEGIN CERTIFICATE-----\n"

    monkeypatch.setattr("harbor_capsem.environment.build_and_stage_oci_image", _fake_build)
    ctrl = FakeCapsemController(tmp_path / "guest")
    env = CapsemEnvironment(df_dir, "df-t", "df-t__1", tp, EnvironmentConfig(), controller=ctrl)
    await env.start(force_build=True)
    assert ctrl.started_vms[0]["image"] == "127.0.0.1:5055/capsem-inspect-build:test"
    assert ctrl.started_vms[0]["registry_ca_pem"] == "-----BEGIN CERTIFICATE-----\n"

    src_file = tmp_path / "solve.sh"
    src_file.write_text("#!/bin/sh\necho solved\n", encoding="utf-8")
    os.chmod(src_file, 0o755)
    await env.upload_file(src_file, "/workspace/solve.sh")
    assert ctrl.uploads["/workspace/solve.sh"] == b"#!/bin/sh\necho solved\n"
    assert any("chmod 755 /workspace/solve.sh" in cmd for cmd, _ in ctrl.commands)

    dl_file = tmp_path / "downloaded_solve.sh"
    await env.download_file("/workspace/solve.sh", dl_file)
    assert dl_file.read_text(encoding="utf-8") == "#!/bin/sh\necho solved\n"

    src_tree = tmp_path / "src_tree"
    (src_tree / "nested").mkdir(parents=True)
    (src_tree / "nested" / "hello.txt").write_text("tar-fidelity-ok\n", encoding="utf-8")
    (src_tree / "link.txt").symlink_to("nested/hello.txt")
    await env.upload_dir(src_tree, "/workspace/uploaded_tree")

    dst_tree = tmp_path / "dst_tree"
    await env.download_dir("/workspace/uploaded_tree", dst_tree)
    assert (dst_tree / "nested" / "hello.txt").read_text(encoding="utf-8") == "tar-fidelity-ok\n"
    assert (dst_tree / "link.txt").is_symlink()
    await env.stop(delete=True)


async def test_concurrent_start_builds_isolated_without_os_environ_mutation(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    env_vars = (
        "CAPSEM_INSPECT_HOST_BUILD",
        "CAPSEM_INSPECT_ALLOWED_HOST_PATHS",
        "CAPSEM_INSPECT_BUILD_NETWORK",
    )
    for k in env_vars:
        monkeypatch.delenv(k, raising=False)
    assert _ib.is_host_build_enabled() is False
    dir_a, dir_b = tmp_path / "conc_a", tmp_path / "conc_b"
    for d in (dir_a, dir_b):
        d.mkdir()
        (d / "Dockerfile").write_text("FROM busybox:latest\n", encoding="utf-8")
    a_entered, b_entered = asyncio.Event(), asyncio.Event()
    seen_specs: dict[str, Mapping[str, Any]] = {}

    async def _concurrent_build(spec: Mapping[str, Any]) -> tuple[str, str]:
        ctx_name = Path(spec["context"]).name
        seen_specs[ctx_name] = spec
        assert "CAPSEM_INSPECT_ALLOWED_HOST_PATHS" not in os.environ
        assert _ib.is_host_build_enabled() is True
        if ctx_name == "conc_a":
            a_entered.set()
            await b_entered.wait()
        else:
            b_entered.set()
            await asyncio.sleep(0.01)
            assert _ib.is_host_build_enabled() is True
        return f"127.0.0.1:5055/{ctx_name}:latest", "-----BEGIN CERTIFICATE-----\n"

    monkeypatch.setattr("harbor_capsem.environment.build_and_stage_oci_image", _concurrent_build)
    tp_a, ctrl_a = _make_trial_paths(tmp_path / "ta"), FakeCapsemController(tmp_path / "ga")
    env_a = CapsemEnvironment(
        dir_a, "conc-a", "conc-a__1", tp_a, EnvironmentConfig(), controller=ctrl_a
    )

    async def _init_and_start_b() -> CapsemEnvironment:
        await a_entered.wait()
        tp_b, ctrl_b = _make_trial_paths(tmp_path / "tb"), FakeCapsemController(tmp_path / "gb")
        cfg_b = EnvironmentConfig().model_copy(update={"allow_internet": False})
        eb = CapsemEnvironment(dir_b, "conc-b", "conc-b__1", tp_b, cfg_b, controller=ctrl_b)
        await eb.start(force_build=True)
        return eb

    _, env_b = await asyncio.gather(env_a.start(force_build=True), _init_and_start_b())
    spec_a, spec_b = seen_specs["conc_a"], seen_specs["conc_b"]
    assert (spec_a["context"], spec_a["network"]) == (str(dir_a.resolve()), "default")
    assert (spec_b["context"], spec_b["network"]) == (str(dir_b.resolve()), "none")
    await env_a.stop(delete=True)
    await env_b.stop(delete=True)
    assert _ib.is_host_build_enabled() is False
