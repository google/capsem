"""`CapsemSandboxEnvironment` initialization, properties, and teardown tests."""

from __future__ import annotations

import asyncio
import time
from pathlib import Path
from typing import Any, cast

import inspect_capsem.sandbox as sb
import pytest
from capsem_integrations._controller import CommandResult
from inspect_capsem import CapsemSandboxConfig, CapsemSandboxEnvironment

from .helpers import LocalFakeCapsemController, Scripted, init_env


async def test_environment_properties_and_connection() -> None:
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
    assert "capsem.yaml" not in CapsemSandboxEnvironment.config_files()
    assert CapsemSandboxEnvironment.is_docker_compatible()
    assert CapsemSandboxEnvironment.default_concurrency() == 4
    container = CapsemSandboxEnvironment("vm-1", ctrl, execution_mode="container")
    conn = await container.connection(user="bob")
    assert conn.type == "capsem"
    assert conn.command == "capsem shell vm-1"
    assert conn.container is None


async def test_sample_init_container_and_vm_modes() -> None:
    ctrl = Scripted()
    env = await init_env(
        ctrl,
        CapsemSandboxConfig(
            execution_mode="container",
            image="ubuntu:24.04",
            healthcheck={"test": ["CMD-SHELL", "true"], "retries": 1, "interval": "10ms"},
        ),
    )
    assert (env.vm_id, env.execution_mode) == ("vm-s", "container")
    assert any("sh -c true" in c for c in ctrl.commands)
    await env.cleanup()
    assert ctrl.stopped == ["vm-s"]

    ctrl = Scripted()
    env = await init_env(ctrl, CapsemSandboxConfig(environment={"FOO": "bar"}))
    assert env.vm_id == "vm-s"
    assert env.execution_mode == "vm"
    assert ctrl.started[0]["env"] == {"FOO": "bar"}
    await env.cleanup()
    assert ctrl.stopped == ["vm-s"]


async def test_failed_bake_tears_down_what_init_created() -> None:
    def hang(command: str) -> CommandResult:
        raise TimeoutError(command)

    ctrl = Scripted([("test -x", hang)])
    with pytest.raises(TimeoutError):
        await init_env(ctrl, CapsemSandboxConfig())
    assert ctrl.stopped == ["vm-s"]


async def test_failed_init_teardown_does_not_block_event_loop(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    def hang(command: str) -> CommandResult:
        raise TimeoutError(command)

    class SlowStop(Scripted):
        async def stop_vm(self, vm_id: str) -> None:
            await asyncio.sleep(0.2)
            if vm_id == "vm-s" and self.stopped:
                raise RuntimeError("second stop fails")
            await super().stop_vm(vm_id)

    ctrl, ticks = SlowStop([("test -x", hang)]), []

    async def ticker() -> None:
        while True:
            ticks.append(time.monotonic())
            await asyncio.sleep(0.01)

    tick_task = asyncio.create_task(ticker())
    monkeypatch.setattr(sb, "SdkCapsemController", lambda: ctrl)
    try:
        for _ in range(2):
            with pytest.raises(TimeoutError):
                await CapsemSandboxEnvironment.sample_init("t", CapsemSandboxConfig(), {})
    finally:
        tick_task.cancel()
    assert ctrl.stopped == ["vm-s"] and len(ticks) > 10


async def test_failed_init_teardown_is_bounded(monkeypatch: pytest.MonkeyPatch) -> None:
    def hang(command: str) -> CommandResult:
        raise TimeoutError(command)

    class HungStop(Scripted):
        async def stop_vm(self, vm_id: str) -> None:
            del vm_id
            await asyncio.sleep(10.0)

    monkeypatch.setattr(sb, "_INIT_TEARDOWN_TIMEOUT_SECS", 0.1)
    t0 = time.monotonic()
    with pytest.raises(TimeoutError, match="test -x"):
        await init_env(HungStop([("test -x", hang)]), CapsemSandboxConfig())
    assert time.monotonic() - t0 < 5


async def test_sample_init_cancellation_cleans_up_started_vm(
    monkeypatch: pytest.MonkeyPatch,
    caplog: pytest.LogCaptureFixture,
) -> None:
    started_vms: list[str] = []
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
    coro = CapsemSandboxEnvironment.sample_init("t", CapsemSandboxConfig(), {})
    task = asyncio.create_task(coro)
    await asyncio.wait_for(entered_bake.wait(), timeout=2.0)
    task.cancel()
    with pytest.raises(asyncio.CancelledError):
        await task
    assert started_vms == ["vm-1"] and ctrl.stopped == ["vm-1"]

    entered_start, finish_start = asyncio.Event(), asyncio.Event()

    class CancelDuringStartController(Scripted):
        def __init__(self, *, fail_stop: bool = False) -> None:
            super().__init__()
            self.fail_stop = fail_stop

        async def start_vm(self, **_: Any) -> str:
            entered_start.set()
            await finish_start.wait()
            return "vm-during-start"

        async def stop_vm(self, vm_id: str) -> None:
            if self.fail_stop:
                raise RuntimeError("stop after cancel failed")
            await super().stop_vm(vm_id)

    cfg = CapsemSandboxConfig()
    ctrl_start_ok = CancelDuringStartController(fail_stop=False)
    monkeypatch.setattr(sb, "SdkCapsemController", lambda: ctrl_start_ok)
    t_start = asyncio.create_task(CapsemSandboxEnvironment.sample_init("t", cfg, {}))
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
    t_start_fail = asyncio.create_task(CapsemSandboxEnvironment.sample_init("t", cfg, {}))
    await asyncio.wait_for(entered_start.wait(), timeout=2.0)
    t_start_fail.cancel()
    finish_start.set()
    with caplog.at_level("WARNING", logger="inspect_capsem"), pytest.raises(asyncio.CancelledError):
        await t_start_fail
    assert "Failed to stop Capsem VM vm-during-start after cancelled start_vm" in caplog.text

    entered_start.clear()
    finish_start.clear()
    monkeypatch.setattr(sb, "_INIT_TEARDOWN_TIMEOUT_SECS", 0.05)
    ctrl_start_hung = CancelDuringStartController(fail_stop=False)
    monkeypatch.setattr(sb, "SdkCapsemController", lambda: ctrl_start_hung)
    t_start_hung = asyncio.create_task(CapsemSandboxEnvironment.sample_init("t", cfg, {}))
    await asyncio.wait_for(entered_start.wait(), timeout=2.0)
    t_start_hung.cancel()
    with pytest.raises(asyncio.CancelledError):
        await t_start_hung
    assert ctrl_start_hung.stopped == []


async def test_controller_closed_on_sample_cleanup_and_failed_init(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Per-sample controller is closed in sample_cleanup and when sample_init fails."""
    created_controllers: list[LocalFakeCapsemController] = []

    def make_controller() -> LocalFakeCapsemController:
        c = LocalFakeCapsemController(tmp_path)
        c.fail_start_vm = len(created_controllers) == 1
        created_controllers.append(c)
        return c

    monkeypatch.setattr(sb, "SdkCapsemController", make_controller)
    envs = await CapsemSandboxEnvironment.sample_init(
        "close_ok", CapsemSandboxConfig(working_dir=str(tmp_path)), {}
    )
    assert len(created_controllers) == 1 and created_controllers[0].close_count == 0
    await CapsemSandboxEnvironment.sample_cleanup("close_ok", None, envs, interrupted=False)
    assert created_controllers[0].close_count == 1

    env = envs["default"]
    assert isinstance(env, CapsemSandboxEnvironment)
    created_controllers[0].fail_stop_vm = True
    await env.cleanup()

    with pytest.raises(RuntimeError, match="start_vm failed"):
        await CapsemSandboxEnvironment.sample_init("close_fail", CapsemSandboxConfig(), {})
    assert len(created_controllers) == 2 and created_controllers[1].close_count == 1


async def test_task_init_and_task_cleanup_isolation_and_retry(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
) -> None:
    """task_cleanup raises NotImplementedError on cleanup=False and scopes cleanup=True by task."""
    await CapsemSandboxEnvironment.task_init("t", None)

    controller = LocalFakeCapsemController(tmp_path)
    monkeypatch.setattr(sb, "SdkCapsemController", lambda: controller)

    envs_a = await CapsemSandboxEnvironment.sample_init(
        task_name="task_a", config=CapsemSandboxConfig(working_dir=str(tmp_path)), metadata={}
    )
    envs_b = await CapsemSandboxEnvironment.sample_init(
        task_name="task_b", config=CapsemSandboxConfig(working_dir=str(tmp_path)), metadata={}
    )
    vm_a = cast(CapsemSandboxEnvironment, envs_a["default"]).vm_id
    vm_b = cast(CapsemSandboxEnvironment, envs_b["default"]).vm_id
    vm_untagged = await controller.start_vm(cpu_count=1, ram_gb=1)
    env_untagged = CapsemSandboxEnvironment(
        vm_id=vm_untagged, controller=cast(Any, controller), working_dir=str(tmp_path)
    )

    with pytest.raises(NotImplementedError, match=r"Hypervisor\.managed\(\).*cleanup=False"):
        await CapsemSandboxEnvironment.task_cleanup("task_b", None, cleanup=False)
    assert vm_b in controller.stopped_vms and controller.close_count == 1
    assert vm_a not in controller.stopped_vms and vm_untagged not in controller.stopped_vms

    await CapsemSandboxEnvironment.task_cleanup("task_a", None, cleanup=True)
    assert vm_a in controller.stopped_vms and controller.close_count == 2
    assert vm_untagged not in controller.stopped_vms

    await CapsemSandboxEnvironment.task_cleanup(cast(Any, None), None, cleanup=True)
    assert vm_untagged in controller.stopped_vms and env_untagged.vm_id == vm_untagged

    class FlakyStopController(Scripted):
        def __init__(self) -> None:
            super().__init__()
            self.fail_stops = 1

        async def stop_vm(self, vm_id: str) -> None:
            if self.fail_stops > 0:
                self.fail_stops -= 1
                raise RuntimeError("transient gateway error")
            await super().stop_vm(vm_id)

    flaky_ctrl = FlakyStopController()
    env_flaky = CapsemSandboxEnvironment(
        "v-flaky", flaky_ctrl, task_name="t-flaky", owns_controller=True
    )
    with caplog.at_level("WARNING", logger="inspect_capsem"):
        await CapsemSandboxEnvironment.sample_cleanup(
            "t-flaky", None, {"default": env_flaky}, interrupted=False
        )
    assert "Failed to stop Capsem VM v-flaky" in caplog.text
    assert "v-flaky" not in flaky_ctrl.stopped and flaky_ctrl.closed

    await CapsemSandboxEnvironment.task_cleanup("t-flaky", None, cleanup=True)
    assert "v-flaky" in flaky_ctrl.stopped and flaky_ctrl.closed

    always_fail_ctrl = FlakyStopController()
    always_fail_ctrl.fail_stops = 5
    env_fail = CapsemSandboxEnvironment(
        "v-always-fail", always_fail_ctrl, task_name="t-fail", owns_controller=True
    )
    await CapsemSandboxEnvironment.task_cleanup("t-fail", None, cleanup=True)
    assert always_fail_ctrl.closed
    assert env_fail._instance_id not in CapsemSandboxEnvironment._active_environments
