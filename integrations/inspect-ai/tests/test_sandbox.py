"""`CapsemSandboxEnvironment` lifecycle, execution, and file handling tests."""

from __future__ import annotations

import asyncio
import os
import pwd
import shlex
import subprocess
import time
from pathlib import Path
from types import SimpleNamespace
from typing import Any, cast

import inspect_capsem.containers.runtime as runtime_mod
import inspect_capsem.sandbox as sb
import pytest
from inspect_ai.util import OutputLimitExceededError
from inspect_capsem import (
    CapsemSandboxConfig,
    CapsemSandboxEnvironment,
    CommandResult,
    SdkCapsemController,
)

from .conftest import (
    LocalFakeCapsemController,
    Scripted,
    _host_timeout_skips,
    _run_inspect_self_check,
    env_for,
    fail,
    ok,
    run_init,
)

sb_mod = sb


def _container_script(inner: str) -> str:
    env = CapsemSandboxEnvironment(
        "vm-1", cast(Any, object()), container_id="cnt-1", execution_mode="container"
    )
    try:
        argv = shlex.split(env._wrap_target_command(inner))
    finally:
        CapsemSandboxEnvironment._active_environments.pop(env._instance_id, None)
    assert argv[:5] == ["docker", "exec", "cnt-1", "bash", "-c"]
    return argv[5]


def _run(script: str, home: str | None, bin_dir: Path) -> str:
    env = {k: v for k, v in os.environ.items() if k != "HOME"}
    env["PATH"] = f"{bin_dir}:{env.get('PATH', '/usr/bin:/bin')}"
    if home is not None:
        env["HOME"] = home
    out = subprocess.run(
        ["bash", "-c", script], env=env, capture_output=True, text=True, check=True
    )
    return out.stdout.strip()


@pytest.fixture
def root_id(tmp_path: Path) -> Path:
    """Fake `id` reporting uid 0, whose home is always in /etc/passwd."""
    fake = tmp_path / "id"
    fake.write_text("#!/bin/sh\necho 0\n")
    fake.chmod(0o755)
    return tmp_path


def _passwd_home(uid: int) -> str:
    return pwd.getpwuid(uid).pw_dir


@pytest.mark.parametrize("leaked", ["/", None])
def test_container_exec_restores_passwd_home(leaked: str | None, root_id: Path) -> None:
    script = _container_script('echo "$HOME"')
    assert _run(script, leaked, root_id) == _passwd_home(0)


def test_container_exec_keeps_explicit_home(root_id: Path) -> None:
    script = _container_script('echo "$HOME"')
    assert _run(script, "/custom/home", root_id) == "/custom/home"


def test_vm_exec_is_not_wrapped() -> None:
    env = CapsemSandboxEnvironment("vm-1", cast(Any, object()), execution_mode="vm")
    try:
        assert env._wrap_target_command("echo hi") == "echo hi"
    finally:
        CapsemSandboxEnvironment._active_environments.pop(env._instance_id, None)


def test_environment_properties_and_connection() -> None:
    ctrl = Scripted()
    original = sb.SdkCapsemController
    cast(Any, sb).SdkCapsemController = lambda: ctrl
    try:
        default = CapsemSandboxEnvironment("vm-2", execution_mode="vm")
        assert default.vm_id == "vm-2" and default._controller is ctrl
        assert default.execution_mode == "vm"
    finally:
        cast(Any, sb).SdkCapsemController = original
    assert "Dockerfile" in CapsemSandboxEnvironment.config_files()
    assert "Containerfile" in CapsemSandboxEnvironment.config_files()
    assert "capsem.yaml" not in CapsemSandboxEnvironment.config_files()
    assert CapsemSandboxEnvironment.is_docker_compatible()
    assert CapsemSandboxEnvironment.default_concurrency() == 4
    container = CapsemSandboxEnvironment(
        "vm-1", ctrl, container_id="c1", execution_mode="container"
    )
    conn = asyncio.run(container.connection(user="bob"))
    assert conn.command == "capsem exec vm-1 -- docker exec -it -u bob c1 bash"
    conn = asyncio.run(env_for(ctrl).connection())
    assert conn.command == "capsem exec vm-s -- bash" and conn.container == "vm-s"


def test_sample_init_container_and_vm_modes() -> None:
    ctrl = Scripted([("docker run -d", ok("cid-123\n"))])
    env = run_init(
        ctrl,
        CapsemSandboxConfig(
            execution_mode="container",
            healthcheck={"test": ["CMD-SHELL", "true"], "retries": 1, "interval": "1s"},
        ),
    )
    assert (env.vm_id, env.container_id) == ("vm-s", "cid-123")
    assert any("docker exec cid-123 sh -c true" in c for c in ctrl.commands)
    asyncio.run(env.cleanup())
    assert ctrl.stopped == ["vm-s"]

    ctrl = Scripted()
    env = run_init(ctrl, CapsemSandboxConfig())
    assert env.container_id is None


def test_container_start_failures(tmp_path: Path) -> None:
    for rules, error in (
        ([("docker info", fail(stderr="daemon down"))], "not ready"),
        ([("docker run -d", fail(stderr="no image"))], "Failed to start container"),
    ):
        ctrl = Scripted(rules)
        with pytest.raises(RuntimeError, match=error):
            run_init(ctrl, CapsemSandboxConfig(execution_mode="container", init=True))
        assert ctrl.stopped == ["vm-s"]

    ctrl = Scripted([("docker run -d", ok("cid-hc\n")), ("docker exec cid-hc sh -c", fail())])
    with pytest.raises(RuntimeError, match="failed healthcheck"):
        run_init(
            ctrl,
            CapsemSandboxConfig(
                execution_mode="container",
                healthcheck={"test": "check_ready", "retries": 1},
            ),
        )
    assert ctrl.stopped == ["vm-s"]

    ctrl = Scripted()
    with pytest.raises(FileNotFoundError):
        run_init(
            ctrl,
            CapsemSandboxConfig(dockerfile=str(tmp_path / "missing")),
        )
    assert ctrl.stopped == ["vm-s"]

    dockerfile = tmp_path / "Dockerfile"
    dockerfile.write_text("FROM scratch\n")
    ctrl = Scripted([("docker run -d", ok(""))])
    env = run_init(ctrl, CapsemSandboxConfig(dockerfile=str(dockerfile)))
    assert env.container_id is not None and env.container_id.startswith("inspect-")
    assert any("capsem-inspect-" in c and "docker run -d" in c for c in ctrl.commands)


def test_failed_bake_tears_down_what_init_created(tmp_path: Path) -> None:
    def hang(command: str) -> CommandResult:
        raise TimeoutError(command)

    ctrl = Scripted([("test -x", hang)])
    with pytest.raises(TimeoutError):
        run_init(ctrl, CapsemSandboxConfig(execution_mode="vm"))
    assert ctrl.stopped == ["vm-s"]

    host_vol = tmp_path / "vol"
    host_vol.mkdir()
    (host_vol / "data.txt").write_text("payload")

    ctrl_cnt = Scripted([("docker run -d", ok("cid-9\n")), ("test -x", hang)])
    with pytest.raises(TimeoutError):
        run_init(
            ctrl_cnt,
            CapsemSandboxConfig(
                execution_mode="container",
                init=True,
                volumes=(f"{host_vol}:/mnt/vol",),
            ),
        )
    assert ctrl_cnt.stopped == ["vm-s"]


def test_failed_init_teardown_does_not_block_event_loop(monkeypatch: pytest.MonkeyPatch) -> None:
    def hang(command: str) -> CommandResult:
        raise TimeoutError(command)

    class SlowStop(Scripted):
        async def stop_vm(self, vm_id: str) -> None:
            await asyncio.sleep(0.2)
            if vm_id == "vm-s" and self.stopped:
                raise RuntimeError("second stop fails")
            await super().stop_vm(vm_id)

    ctrl = SlowStop([("test -x", hang)])
    ticks: list[float] = []

    async def ticker() -> None:
        while True:
            ticks.append(time.monotonic())
            await asyncio.sleep(0.01)

    async def main() -> None:
        tick_task = asyncio.create_task(ticker())
        monkeypatch.setattr(sb, "SdkCapsemController", lambda: ctrl)
        try:
            with pytest.raises(TimeoutError):
                await CapsemSandboxEnvironment.sample_init("t", CapsemSandboxConfig(), {})
            # A failing teardown is logged; the original error still propagates.
            with pytest.raises(TimeoutError):
                await CapsemSandboxEnvironment.sample_init("t", CapsemSandboxConfig(), {})
        finally:
            tick_task.cancel()

    asyncio.run(main())
    assert ctrl.stopped == ["vm-s"]
    assert len(ticks) > 10


def test_failed_init_teardown_is_bounded(monkeypatch: pytest.MonkeyPatch) -> None:
    def hang(command: str) -> CommandResult:
        raise TimeoutError(command)

    class HungStop(Scripted):
        async def stop_vm(self, vm_id: str) -> None:
            del vm_id
            await asyncio.sleep(10.0)

    monkeypatch.setattr(sb, "_INIT_TEARDOWN_TIMEOUT_SECS", 0.1)
    t0 = time.monotonic()
    with pytest.raises(TimeoutError, match="test -x"):
        run_init(HungStop([("test -x", hang)]), CapsemSandboxConfig())
    assert time.monotonic() - t0 < 5


def test_cli_cleanup_branches(caplog: pytest.LogCaptureFixture) -> None:
    class PartialFailStop(Scripted):
        async def stop_vm(self, vm_id: str, *, timeout: float | None = None) -> None:
            del timeout
            if vm_id == "inspect-capsem-fail":
                raise RuntimeError("stop failed on first vm")
            await super().stop_vm(vm_id)

    ctrl = PartialFailStop()
    ctrl.vms = [
        {"id": "inspect-capsem-fail"},
        {"id": "inspect-capsem-2"},
        {"id": "keep", "name": "user-vm"},
        {"name": ""},
    ]
    owned = CapsemSandboxEnvironment("vm-owned", ctrl, execution_mode="vm")
    other = CapsemSandboxEnvironment("vm-other", ctrl, execution_mode="vm")
    original = sb.SdkCapsemController
    cast(Any, sb).SdkCapsemController = lambda: ctrl
    try:
        asyncio.run(CapsemSandboxEnvironment.cli_cleanup("vm-owned"))
        assert "vm-owned" in ctrl.stopped
        asyncio.run(CapsemSandboxEnvironment.cli_cleanup("unmatched-id"))
        assert "keep" not in ctrl.stopped
        assert ctrl.commands == []
        asyncio.run(CapsemSandboxEnvironment.cli_cleanup("keep"))
        assert "keep" in ctrl.stopped
        ctrl.stopped.remove("keep")
        with caplog.at_level("WARNING", logger="inspect_capsem.sandbox"):
            asyncio.run(CapsemSandboxEnvironment.cli_cleanup(None))
        assert "Failed to clean up Capsem VM inspect-capsem-fail" in caplog.text
        assert "inspect-capsem-2" in ctrl.stopped
        assert "keep" not in ctrl.stopped
        assert ctrl.commands == []

        class Down(Scripted):
            async def list_vms(self) -> list[dict[str, Any]]:
                raise RuntimeError("down")

        cast(Any, sb).SdkCapsemController = Down
        caplog.clear()
        with caplog.at_level("WARNING", logger="inspect_capsem.sandbox"):
            asyncio.run(CapsemSandboxEnvironment.cli_cleanup(None))
        assert "Controller list_vms failed during cli_cleanup" in caplog.text
    finally:
        cast(Any, sb).SdkCapsemController = original
    del owned, other


def test_sweep_deletes_only_idle_managed_vms(monkeypatch: pytest.MonkeyPatch) -> None:
    class Flaky(Scripted):
        async def stop_vm(self, vm_id: str) -> None:
            if vm_id == "e":
                raise RuntimeError("delete failed")
            await super().stop_vm(vm_id)

    ctrl = Flaky()
    ctrl.vms = [
        {"id": "a", "name": "inspect-capsem-1", "status": "Stopped", "persistent": True},
        {"id": "b", "name": "inspect-capsem-2", "status": "Running"},
        {"id": "c", "name": "user-vm", "status": "Stopped"},
        {"id": "d", "name": "inspect-capsem-3", "status": "Defunct"},
        {"id": "e", "name": "inspect-capsem-4", "status": "Stopped"},
        {"name": "inspect-capsem-5", "status": "Stopped"},
    ]
    assert asyncio.run(sb.sweep_leftover_vms(ctrl)) == ["a", "d"]
    assert ctrl.stopped == ["a", "d"]

    closed: list[bool] = []

    class Closing(Scripted):
        async def close(self) -> None:
            closed.append(True)

    closing = Closing()
    closing.vms = [{"id": "z", "name": "inspect-capsem-z", "status": "Stopped"}]
    monkeypatch.setattr(sb, "SdkCapsemController", lambda: closing)
    asyncio.run(CapsemSandboxEnvironment.task_init("t", None))
    assert closing.stopped == ["z"] and closed == [True]

    class Down(Scripted):
        async def list_vms(self) -> list[dict[str, Any]]:
            raise RuntimeError("down")

    assert asyncio.run(sb.sweep_leftover_vms(Down())) == []

    def broken_factory() -> Any:
        raise RuntimeError("no controller")

    monkeypatch.setattr(sb, "SdkCapsemController", broken_factory)
    asyncio.run(CapsemSandboxEnvironment.task_init("t", None))


def test_oci_single_service_sample_init_and_cleanup() -> None:
    from capsem.models import ExecOutput, ExecOutputEncoding

    class OciSession:
        id = "vm-oci"

        def __init__(self) -> None:
            self.commands: list[str] = []
            self.deleted = False

        async def exec(self, command: str, *, timeout_secs: int | None = None) -> Any:
            del timeout_secs
            self.commands.append(command)
            return SimpleNamespace(
                exit_code=0,
                stdout=ExecOutput(data="ok\n", encoding=ExecOutputEncoding.UTF8),
                stderr=ExecOutput(data="", encoding=ExecOutputEncoding.UTF8),
                truncated=False,
            )

        async def delete(self) -> None:
            self.deleted = True

    session = OciSession()
    create_calls: list[dict[str, Any]] = []

    class OciHv:
        async def create(
            self,
            *,
            name: str | None = None,
            profile: Any = None,
            cpus: int | None = None,
            memory: int | None = None,
            image: str | None = None,
            command: list[str] | None = None,
            env: dict[str, str] | None = None,
        ) -> Any:
            create_calls.append(
                {
                    "name": name,
                    "profile": profile,
                    "cpus": cpus,
                    "memory": memory,
                    "image": image,
                    "command": command,
                    "env": env,
                }
            )
            return session

        async def close(self) -> None:
            return None

    sdk_ctrl = SdkCapsemController(hypervisor=OciHv())
    try:
        assert sb._can_use_oci_image_create(
            CapsemSandboxConfig(execution_mode="container", image="python:3.11-slim")
        )
        assert not sb._can_use_oci_image_create(
            CapsemSandboxConfig(execution_mode="container", image="python:3.11-slim", init=True)
        )

        env = run_init(
            sdk_ctrl,
            CapsemSandboxConfig(
                execution_mode="container",
                image="python:3.11-slim",
                command="echo boot",
                environment={"FOO": "bar"},
            ),
        )
        assert env.vm_id == "vm-oci"
        assert env.container_id == "workload"
        assert create_calls[0]["image"] == "docker://python:3.11-slim"
        assert create_calls[0]["command"] == ["sh", "-c", "echo boot"]
        assert create_calls[0]["env"]["FOO"] == "bar"
        assert any(
            "nsenter -t" in c
            and "bundle/rootfs/workspace" in c
            and "/proc/mounts" in c
            and runtime_mod._OCI_RUNC_EXEC_ROOT in c
            and runtime_mod._OCI_RUNC_EXEC in c
            for c in session.commands
        )

        conn = asyncio.run(env.connection())
        assert runtime_mod._OCI_RUNC_EXEC in conn.command
        conn_user = asyncio.run(env.connection(user="bob"))
        assert f"{runtime_mod._OCI_RUNC_EXEC_PREFIX} -u bob workload bash" in conn_user.command

        res = asyncio.run(env.exec(["echo", "hello"]))
        assert res.success and any(
            c.startswith(runtime_mod._OCI_RUNC_EXEC) and "echo hello" in c for c in session.commands
        )
        res_root = asyncio.run(env.exec(["id"], user="root"))
        assert res_root.success and any(
            c.startswith(runtime_mod._OCI_RUNC_EXEC_ROOT) and "id" in c for c in session.commands
        )
        res_bob = asyncio.run(env.exec(["id"], user="bob"))
        assert res_bob.success and any(
            c.startswith(runtime_mod._OCI_RUNC_EXEC_ROOT) and "su -m bob" in c
            for c in session.commands
        )

        asyncio.run(env.cleanup())
        assert session.deleted
        assert not any("docker rm -f" in c for c in session.commands)
    finally:
        asyncio.run(sdk_ctrl.close())

    # Default command=None includes the ENTRYPOINT relaunch fallback in prepare_oci_workload_container.
    ctrl_default_cmd = Scripted()
    asyncio.run(
        runtime_mod.prepare_oci_workload_container(
            ctrl_default_cmd, "vm-1", CapsemSandboxConfig().to_container_spec()
        )
    )
    assert any("/root/.capsem-image/launch.py" in c for c in ctrl_default_cmd.commands)

    # Failure in prepare_oci_workload_container raises RuntimeError.
    ctrl_fail = Scripted([("workload.pid", fail(stderr="pid timeout"))])
    with pytest.raises(RuntimeError, match="pid timeout"):
        asyncio.run(
            runtime_mod.prepare_oci_workload_container(
                ctrl_fail, "vm-1", CapsemSandboxConfig().to_container_spec()
            )
        )


def test_process_owned_vms_and_leftover_sweep_status_filtering(
    monkeypatch: pytest.MonkeyPatch,
    caplog: pytest.LogCaptureFixture,
) -> None:
    ctrl = Scripted()
    ctrl.vms = [
        {"id": "v-stop", "name": "inspect-capsem-1", "status": "Stopped"},
        {"id": "v-fail", "name": "inspect-capsem-2", "status": "Failed"},
        {"id": "v-boot", "name": "inspect-capsem-3", "status": "Booting"},
        {"id": "v-start", "name": "inspect-capsem-4", "status": "Starting"},
        {"id": "v-pause", "name": "inspect-capsem-5", "status": "Paused"},
        {"id": "v-own", "name": "inspect-capsem-6", "status": "Stopped"},
    ]
    monkeypatch.setattr(sb, "SdkCapsemController", lambda: ctrl)
    sb._register_process_owned_vm("v-own")
    try:
        swept = asyncio.run(sb.sweep_leftover_vms(ctrl))
        assert swept == ["v-stop", "v-fail"]
        assert "v-boot" not in ctrl.stopped
        assert "v-start" not in ctrl.stopped
        assert "v-pause" not in ctrl.stopped
        assert "v-own" not in ctrl.stopped
    finally:
        sb._atexit_sweep_process_owned_vms()
        assert "v-own" in ctrl.stopped

    # task_cleanup stops only entries matching task_name and leaves untagged (None) entries intact.
    ctrl2 = Scripted()
    monkeypatch.setattr(sb, "SdkCapsemController", lambda: ctrl2)
    env = CapsemSandboxEnvironment("v-env", ctrl2, task_name="t")
    env_untagged = CapsemSandboxEnvironment("v-untagged", ctrl2, task_name=None)
    sb._register_process_owned_vm("v-env", task_name="t")
    sb._register_process_owned_vm("v-orphan", task_name="t")
    sb._register_process_owned_vm("v-untagged", task_name=None)
    asyncio.run(CapsemSandboxEnvironment.task_cleanup("other", None, cleanup=False))
    assert ctrl2.stopped == []
    asyncio.run(CapsemSandboxEnvironment.task_cleanup("t", None, cleanup=True))
    assert set(ctrl2.stopped) == {"v-env", "v-orphan"}
    assert "v-untagged" in sb._PROCESS_OWNED_VMS
    asyncio.run(CapsemSandboxEnvironment.task_cleanup(cast(Any, None), None, cleanup=True))
    assert "v-untagged" in ctrl2.stopped
    del env, env_untagged

    # If stop_vm fails during sample_cleanup, log WARNING and keep vm_id registered so task_cleanup retries.
    class FlakyStopController(Scripted):
        def __init__(self) -> None:
            super().__init__()
            self.fail_next_stop = True

        async def stop_vm(self, vm_id: str, *, timeout: float | None = None) -> None:
            del timeout
            if self.fail_next_stop:
                self.fail_next_stop = False
                raise RuntimeError("transient gateway error")
            await super().stop_vm(vm_id)

    flaky_ctrl = FlakyStopController()
    monkeypatch.setattr(sb, "SdkCapsemController", lambda: flaky_ctrl)
    env_flaky = CapsemSandboxEnvironment("v-flaky", flaky_ctrl, task_name="t-flaky")
    sb._register_process_owned_vm("v-flaky", task_name="t-flaky")
    with caplog.at_level("WARNING", logger="inspect_capsem.sandbox"):
        asyncio.run(
            CapsemSandboxEnvironment.sample_cleanup(
                "t-flaky", None, {"default": env_flaky}, interrupted=False
            )
        )
    assert "Failed to stop Capsem VM v-flaky" in caplog.text
    assert "v-flaky" in sb._PROCESS_OWNED_VMS
    assert "v-flaky" not in flaky_ctrl.stopped
    asyncio.run(CapsemSandboxEnvironment.task_cleanup("t-flaky", None, cleanup=True))
    assert "v-flaky" in flaky_ctrl.stopped
    assert "v-flaky" not in sb._PROCESS_OWNED_VMS


def test_read_file_limits_and_decoding(monkeypatch: pytest.MonkeyPatch) -> None:
    ctrl = Scripted([("SIZE:", ok("SIZE:5000\n"))])
    env = env_for(ctrl)
    monkeypatch.setattr(sb.SandboxEnvironmentLimits, "MAX_READ_FILE_SIZE", 100)
    with pytest.raises(OutputLimitExceededError):
        asyncio.run(env.read_file("big"))
    ctrl.rules = [("SIZE:", ok("SIZE:\n"))]
    ctrl.files["/workspace/big"] = b"x" * 200
    with pytest.raises(OutputLimitExceededError):
        asyncio.run(env.read_file("big"))
    ctrl.files["/workspace/bin"] = b"\xff\xfe"
    with pytest.raises(UnicodeDecodeError):
        asyncio.run(env.read_file("bin"))
    ctrl.files["/workspace/nul"] = b"a\x00b"
    with pytest.raises(UnicodeDecodeError):
        asyncio.run(env.read_file("nul"))
    ctrl.files["/workspace/ok"] = b"fine"
    assert asyncio.run(env.read_file("ok")) == "fine"


def test_exec_edge_cases() -> None:
    ctrl = Scripted()
    env = env_for(ctrl)
    assert asyncio.run(env.exec([])).success
    ctrl.rules = [("./tool", fail(126))]
    assert not asyncio.run(env.exec(["./tool"])).success
    ctrl.rules = [("my_script", fail(126, stderr="custom failure"))]
    assert asyncio.run(env.exec(["bash", "-c", "my_script"])).returncode == 126
    ctrl.rules = [("timeout -k", fail(126, stderr="bash: Permission denied"))]
    with pytest.raises(PermissionError, match="Permission denied"):
        asyncio.run(env.exec(["tool"], timeout=5))
    ctrl.rules = [("timeout -k", CommandResult(0, "x", "", truncated=True))]
    with pytest.raises(OutputLimitExceededError):
        asyncio.run(env.exec(["tool"], timeout=5))

    ctrl.rules = []
    ctrl.commands.clear()
    result = asyncio.run(env.exec(["cat"], input="x" * 20000, env={"A": "1"}, user="bob"))
    assert result.success
    assert any(".capsem_stdin_" in p for p in ctrl.uploads)
    assert len(ctrl.commands) == 1
    script = ctrl.commands[0]
    assert "su -m bob" in script and "export A=1" in script and "timeout -k" not in script
    assert "rm -f /tmp/.capsem_stdin_" in script
    asyncio.run(env.exec(["echo", "y" * 70000]))
    assert any(".capsem_cmd_" in p for p in ctrl.uploads)
    assert len(ctrl.commands) == 2 and "rm -f /tmp/.capsem_cmd_" in ctrl.commands[1]

    # Default non-root self._user runs container exec with -u 0 + su -m even when user=None,
    # and resets USER, LOGNAME, and HOME to the target user's passwd entry while honoring explicit env.
    ctrl.commands.clear()
    env_nonroot = env_for(ctrl, container_id="c1", execution_mode="container", user="developer")
    assert asyncio.run(env_nonroot.exec(["id", "-un"])).success
    assert any(
        c.startswith("docker exec -u 0 c1 bash -c")
        and "su -m developer" in c
        and 'export USER="$__u" LOGNAME="$__u" HOME="${__h:-/home/$__u}"' in c
        for c in ctrl.commands
    )
    nonroot_script, _ = sb._format_exec_command(
        'printf "%s:%s:%s" "$USER" "$LOGNAME" "$HOME"',
        effective_cwd="/",
        env=None,
        user=str(os.getuid()),
        timeout=None,
    )
    su_idx = nonroot_script.index("-s /bin/bash -c ") + len("-s /bin/bash -c ")
    su_body = shlex.split(nonroot_script[su_idx:])[0]
    sub_env = {**os.environ, "HOME": "/root", "USER": "root", "LOGNAME": "root"}
    sub_out = subprocess.run(
        ["bash", "-c", su_body], env=sub_env, capture_output=True, text=True, check=True
    ).stdout
    expected_user = subprocess.run(
        ["id", "-un"], capture_output=True, text=True, check=True
    ).stdout.strip()
    assert sub_out == f"{expected_user}:{expected_user}:{_passwd_home(os.getuid())}"

    override_script, _ = sb._format_exec_command(
        'printf "%s" "$HOME"',
        effective_cwd="/",
        env={"HOME": "/custom/override"},
        user=str(os.getuid()),
        timeout=None,
    )
    su_override_idx = override_script.index("-s /bin/bash -c ") + len("-s /bin/bash -c ")
    su_override_body = shlex.split(override_script[su_override_idx:])[0]
    override_out = subprocess.run(
        ["bash", "-c", su_override_body], env=sub_env, capture_output=True, text=True, check=True
    ).stdout
    assert override_out == "/custom/override"

    ctrl.commands.clear()
    env_uid = env_for(ctrl, container_id="c1", execution_mode="container", user="1000:1000")
    assert asyncio.run(env_uid.exec(["id", "-u"])).success
    assert any(
        c.startswith("docker exec -u 0 c1 bash -c") and "id -un 1000" in c for c in ctrl.commands
    )

    class Slow(Scripted):
        async def exec_in_vm(
            self, vm_id: str, command: str, *, timeout: int = 120
        ) -> CommandResult:
            if "timeout -k" in command:
                raise TimeoutError(f"timed out after {timeout}s")
            return await super().exec_in_vm(vm_id, command, timeout=timeout)

    slow_ctrl = Slow()
    with pytest.raises(TimeoutError):
        asyncio.run(env_for(slow_ctrl).exec(["sleep", "9"], input="x" * 20000, timeout=1))
    assert any(c.startswith("rm -f /tmp/.capsem_stdin_") for c in slow_ctrl.commands)


def test_write_file_refusals() -> None:
    ctrl = Scripted([("IS_DIR", ok("IS_DIR"))])
    with pytest.raises(IsADirectoryError):
        asyncio.run(env_for(ctrl).write_file("d", "x"))
    ctrl = Scripted([("NO_WRITE", CommandResult(13, "", ""))])
    with pytest.raises(PermissionError):
        asyncio.run(env_for(ctrl).write_file("f", b"x"))


def test_sample_init_cancellation_cleans_up_started_vm(
    monkeypatch: pytest.MonkeyPatch,
    caplog: pytest.LogCaptureFixture,
) -> None:
    started_vms: list[str] = []

    async def _run() -> None:
        entered_bake = asyncio.Event()

        class CancelMidInitController(Scripted):
            async def start_vm(self, **_: Any) -> str:
                vid = f"vm-{len(started_vms) + 1}"
                started_vms.append(vid)
                return vid

            async def exec_in_vm(
                self, vm_id: str, command: str, *, timeout: int = 120
            ) -> CommandResult:
                if command.startswith("test -x"):
                    entered_bake.set()
                    await asyncio.sleep(10.0)
                return await super().exec_in_vm(vm_id, command, timeout=timeout)

        ctrl = CancelMidInitController()
        monkeypatch.setattr(sb, "SdkCapsemController", lambda: ctrl)
        task = asyncio.create_task(
            CapsemSandboxEnvironment.sample_init("t", CapsemSandboxConfig(), {})
        )
        await asyncio.wait_for(entered_bake.wait(), timeout=2.0)
        task.cancel()
        with pytest.raises(asyncio.CancelledError):
            await task
        assert started_vms == ["vm-1"]
        assert ctrl.stopped == ["vm-1"]

        # Cancel while start_vm is still awaiting: finishes inside shield and stops VM
        # (and logs a warning if stop_vm fails).
        entered_start = asyncio.Event()
        finish_start = asyncio.Event()

        class CancelDuringStartController(Scripted):
            def __init__(self, *, fail_stop: bool = False) -> None:
                super().__init__()
                self.fail_stop = fail_stop

            async def start_vm(self, **_: Any) -> str:
                entered_start.set()
                await finish_start.wait()
                return "vm-during-start"

            async def stop_vm(self, vm_id: str, *, timeout: float | None = None) -> None:
                del timeout
                if self.fail_stop:
                    raise RuntimeError("stop after cancel failed")
                await super().stop_vm(vm_id)

        ctrl_start_ok = CancelDuringStartController(fail_stop=False)
        monkeypatch.setattr(sb, "SdkCapsemController", lambda: ctrl_start_ok)
        t_start = asyncio.create_task(
            CapsemSandboxEnvironment.sample_init("t", CapsemSandboxConfig(), {})
        )
        await asyncio.wait_for(entered_start.wait(), timeout=2.0)
        t_start.cancel()
        finish_start.set()
        with pytest.raises(asyncio.CancelledError):
            await t_start
        assert ctrl_start_ok.stopped == ["vm-during-start"]

        entered_start.clear()
        finish_start.clear()
        ctrl_start_fail = CancelDuringStartController(fail_stop=True)
        monkeypatch.setattr(sb, "SdkCapsemController", lambda: ctrl_start_fail)
        t_start_fail = asyncio.create_task(
            CapsemSandboxEnvironment.sample_init("t", CapsemSandboxConfig(), {})
        )
        await asyncio.wait_for(entered_start.wait(), timeout=2.0)
        t_start_fail.cancel()
        finish_start.set()
        with (
            caplog.at_level("WARNING", logger="inspect_capsem.sandbox"),
            pytest.raises(asyncio.CancelledError),
        ):
            await t_start_fail
        assert "Failed to stop Capsem VM vm-during-start after cancelled start_vm" in caplog.text

    asyncio.run(_run())


def test_controller_close_lifecycle(monkeypatch: pytest.MonkeyPatch) -> None:
    """task_init and cli_cleanup close temporary controllers."""
    events: list[str] = []

    class ClosingController(Scripted):
        async def list_vms(self) -> list[dict[str, Any]]:
            events.append("list_vms")
            return []

        async def close(self) -> None:
            events.append("close")

    monkeypatch.setattr(sb, "SdkCapsemController", ClosingController)
    asyncio.run(CapsemSandboxEnvironment.task_init("t", None))
    assert events == ["list_vms", "close"]
    events.clear()
    asyncio.run(CapsemSandboxEnvironment.cli_cleanup(None))
    assert events == ["list_vms", "close"]


@pytest.mark.asyncio
async def test_exit_137_sigkill_vs_actual_timeout(tmp_path: Path) -> None:
    """Exit 137 from SIGKILL is not misreported as TimeoutError."""
    controller = LocalFakeCapsemController(tmp_path)
    work_dir = tmp_path / "work"
    work_dir.mkdir()
    env = CapsemSandboxEnvironment(
        vm_id="vm-sigkill",
        controller=cast(Any, controller),
        working_dir=str(work_dir),
        execution_mode="vm",
    )
    try:
        res = await env.exec(["bash", "-c", "kill -KILL $$"], timeout=30)
        assert res.success is False
        assert res.returncode == 137

        with pytest.raises(TimeoutError):
            await env.exec(["bash", "-c", "trap '' TERM; sleep 5"], timeout=1)
    finally:
        await env.cleanup()


@pytest.mark.asyncio
async def test_exec_timeout_none_uses_service_ceiling_and_clean_error_message() -> None:
    """exec(timeout=None) uses EXEC_TIMEOUT_CEILING_SECS (3600s) and never formats 'after Nones'."""
    recorded: list[tuple[str, int]] = []
    raise_mode: str | None = None

    class TimeoutRecordingController:
        async def stop_vm(self, vm_id: str) -> None:
            del vm_id

        async def exec_in_vm(
            self, vm_id: str, command: str, *, timeout: int = 120
        ) -> CommandResult:
            del vm_id
            recorded.append((command, timeout))
            if raise_mode == "timeout_error":
                raise TimeoutError("capsem gateway timeout")
            return CommandResult(exit_code=0, stdout="hi\n", stderr="")

    ctrl = TimeoutRecordingController()
    env = CapsemSandboxEnvironment(
        vm_id="vm-timeout-none",
        controller=cast(Any, ctrl),
        working_dir="/workspace",
        execution_mode="vm",
    )
    try:
        res = await env.exec(["echo", "hi"], timeout=None)
        assert res.success is True
        assert res.stdout == "hi\n"
        assert len(recorded) == 1
        cmd_sent, timeout_sent = recorded[0]
        assert timeout_sent == 3600
        assert "timeout -k 1s" not in cmd_sent

        raise_mode = "timeout_error"
        with pytest.raises(TimeoutError, match=r"^Command timed out after 3600s$") as exc_info:
            await env.exec(["sleep", "9999"], timeout=None)
        assert "Nones" not in str(exc_info.value)
    finally:
        await env.cleanup()


@pytest.mark.asyncio
async def test_cli_cleanup_cleans_out_of_process_vms(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """cli_cleanup queries controller.list_vms() for out-of-process VMs and ignores test/sdk prefixes."""
    controller = LocalFakeCapsemController(tmp_path)
    monkeypatch.setattr(sb_mod, "SdkCapsemController", lambda: controller)

    vid = await controller.start_vm(template="harbor", cpu_count=2, ram_gb=4)
    controller.vms["sdk-vm-user"] = tmp_path / "sdk-vm-user"
    controller.vms["test-vm-user"] = tmp_path / "test-vm-user"
    CapsemSandboxEnvironment._active_environments.clear()

    await CapsemSandboxEnvironment.cli_cleanup(None)
    assert vid in controller.stopped_vms
    assert "sdk-vm-user" not in controller.stopped_vms
    assert "test-vm-user" not in controller.stopped_vms


@pytest.mark.asyncio
async def test_multi_task_cleanup_isolation_and_no_sandbox_cleanup(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    """task_cleanup scopes teardown to task_name and prints surviving VMs on cleanup=False."""
    import inspect_capsem.sandbox as sandbox_mod

    controller = LocalFakeCapsemController(tmp_path)
    monkeypatch.setattr(sb_mod, "SdkCapsemController", lambda: controller)

    envs_a = await CapsemSandboxEnvironment.sample_init(
        task_name="task_a",
        config=CapsemSandboxConfig(execution_mode="vm", working_dir=str(tmp_path)),
        metadata={},
    )
    envs_b = await CapsemSandboxEnvironment.sample_init(
        task_name="task_b",
        config=CapsemSandboxConfig(execution_mode="vm", working_dir=str(tmp_path)),
        metadata={},
    )
    vm_a = cast(CapsemSandboxEnvironment, envs_a["default"]).vm_id
    vm_b = cast(CapsemSandboxEnvironment, envs_b["default"]).vm_id
    vm_untagged = await controller.start_vm(template="code", cpu_count=1, ram_gb=1)
    env_untagged = CapsemSandboxEnvironment(
        vm_id=vm_untagged,
        controller=cast(Any, controller),
        working_dir=str(tmp_path),
        execution_mode="vm",
        task_name=None,
    )
    sandbox_mod._register_process_owned_vm(vm_untagged, task_name=None)
    assert vm_a in sandbox_mod._PROCESS_OWNED_VMS
    assert vm_b in sandbox_mod._PROCESS_OWNED_VMS
    assert vm_untagged in sandbox_mod._PROCESS_OWNED_VMS

    # Cleaning up task_a with cleanup=True stops only task_a's VM and leaves task_b + untagged active.
    await CapsemSandboxEnvironment.task_cleanup("task_a", None, cleanup=True)
    assert vm_a in controller.stopped_vms
    assert vm_b not in controller.stopped_vms
    assert vm_untagged not in controller.stopped_vms
    assert vm_a not in sandbox_mod._PROCESS_OWNED_VMS
    assert vm_b in sandbox_mod._PROCESS_OWNED_VMS
    assert vm_untagged in sandbox_mod._PROCESS_OWNED_VMS
    assert any(e.vm_id == vm_b for e in CapsemSandboxEnvironment._active_environments.values())
    assert any(
        e.vm_id == vm_untagged for e in CapsemSandboxEnvironment._active_environments.values()
    )

    # --no-sandbox-cleanup (cleanup=False) prints the surviving VM ID and cleanup hint,
    # disarms atexit for task_b without stopping vm_b, and closes the controller.
    capsys.readouterr()
    await CapsemSandboxEnvironment.task_cleanup("task_b", None, cleanup=False)
    out_single = capsys.readouterr().out
    assert vm_b in out_single
    assert f"inspect sandbox cleanup capsem {vm_b}" in out_single
    assert vm_b not in controller.stopped_vms
    assert vm_b not in sandbox_mod._PROCESS_OWNED_VMS
    assert not any(e.vm_id == vm_b for e in CapsemSandboxEnvironment._active_environments.values())

    # Multi-VM cleanup=False prints all surviving VM IDs and one cleanup command per ID.
    sandbox_mod._register_process_owned_vm("vm-m1", task_name="task_m")
    sandbox_mod._register_process_owned_vm("vm-m2", task_name="task_m")
    await CapsemSandboxEnvironment.task_cleanup("task_m", None, cleanup=False)
    out_multi = capsys.readouterr().out
    assert "vm-m1, vm-m2" in out_multi
    assert "inspect sandbox cleanup capsem vm-m1" in out_multi
    assert "inspect sandbox cleanup capsem vm-m2" in out_multi

    # Clean up the untagged VM before verifying sweep_process_owned_vms() is empty.
    await env_untagged.cleanup()
    assert await sandbox_mod.sweep_process_owned_vms() == []
    assert vm_b not in controller.stopped_vms

    # cli_cleanup(None) narrows VM teardown to inspect-capsem-* prefix only.
    controller.vms["user-preexisting-vm"] = tmp_path / "user-preexisting-vm"
    await CapsemSandboxEnvironment.cli_cleanup(None)
    assert vm_b in controller.stopped_vms
    assert "user-preexisting-vm" not in controller.stopped_vms
    assert controller.close_count >= 2


@pytest.mark.asyncio
async def test_controller_closed_on_sample_cleanup_and_failed_init(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Per-sample controller is closed in sample_cleanup and when sample_init fails."""
    created_controllers: list[LocalFakeCapsemController] = []

    def make_controller() -> LocalFakeCapsemController:
        c = LocalFakeCapsemController(tmp_path)
        created_controllers.append(c)
        return c

    monkeypatch.setattr(sb_mod, "SdkCapsemController", make_controller)

    envs = await CapsemSandboxEnvironment.sample_init(
        task_name="close_ok",
        config=CapsemSandboxConfig(execution_mode="vm", working_dir=str(tmp_path)),
        metadata={},
    )
    assert len(created_controllers) == 1
    assert created_controllers[0].close_count == 0
    await CapsemSandboxEnvironment.sample_cleanup("close_ok", None, envs, interrupted=False)
    assert created_controllers[0].close_count == 1

    # Idempotent second cleanup when stop_vm raises on already-stopped VM.
    env = envs["default"]
    assert isinstance(env, CapsemSandboxEnvironment)
    created_controllers[0].fail_stop_vm = True
    await env.cleanup()

    # Failed sample_init also closes its controller.
    with pytest.raises(FileNotFoundError):
        await CapsemSandboxEnvironment.sample_init(
            task_name="close_fail",
            config=CapsemSandboxConfig(
                execution_mode="container",
                dockerfile=str(tmp_path / "missing.Dockerfile"),
            ),
            metadata={},
        )
    assert len(created_controllers) == 2
    assert created_controllers[1].close_count == 1


@pytest.mark.asyncio
async def test_self_check_with_local_fake_controller(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """sample_init passes self_check with LocalFakeCapsemController."""
    controller = LocalFakeCapsemController(tmp_path, skip_bake=False)
    monkeypatch.setattr(sb_mod, "SdkCapsemController", lambda: controller)

    guest_work = tmp_path / "guest_work"
    guest_work.mkdir()
    envs = await CapsemSandboxEnvironment.sample_init(
        task_name="self_check_no_host_ws",
        config=CapsemSandboxConfig(execution_mode="vm", working_dir=str(guest_work)),
        metadata={},
    )
    env = envs["default"]
    assert isinstance(env, CapsemSandboxEnvironment)
    try:
        skip = (
            {
                "test_read_and_write_large_file_binary",  # covered by unit tests without multi-MiB host I/O
                "test_exec_input_large",  # covered by unit tests without multi-MiB host stdin staging
                "test_read_file_limit",  # covered by test_read_file_limits_and_decoding with mocked limit
                "test_exec_as_user",  # requires root + `su` inside a real guest VM (covered in test_live.py)
            }
            | _host_timeout_skips()
        )
        results = await _run_inspect_self_check(env, skip=skip)
        failures = {k: v for k, v in results.items() if v is not True}
        assert failures == {}, f"Inspect self_check failures: {failures}"
    finally:
        await CapsemSandboxEnvironment.sample_cleanup(
            task_name="self_check_no_host_ws", config=None, environments=envs, interrupted=False
        )
