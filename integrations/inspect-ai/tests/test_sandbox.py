"""`CapsemSandboxEnvironment` initialization, properties, and teardown tests."""

from __future__ import annotations

import asyncio
import time
from pathlib import Path
from typing import Any, cast

import inspect_capsem._lifecycle as lifecycle_mod
import inspect_capsem.sandbox as sb
import pytest
from inspect_capsem import CapsemSandboxConfig, CapsemSandboxEnvironment
from inspect_capsem._controller import CommandResult

from .helpers import LocalFakeCapsemController, Scripted, env_for, init_env


async def test_environment_properties_and_connection() -> None:
    ctrl = Scripted()
    original = sb.SdkCapsemController
    cast(Any, sb).SdkCapsemController = lambda: ctrl
    try:
        default = CapsemSandboxEnvironment("vm-2")
        assert default.vm_id == "vm-2" and default._controller is ctrl
    finally:
        cast(Any, sb).SdkCapsemController = original
    assert CapsemSandboxEnvironment.config_files() == []
    assert not CapsemSandboxEnvironment.is_docker_compatible()
    assert CapsemSandboxEnvironment.default_concurrency() == 4
    conn = await env_for(ctrl).connection()
    assert conn.type == "capsem"
    assert conn.command == "capsem shell vm-s"
    assert conn.container is None


async def test_sample_init_vm_mode() -> None:
    ctrl = Scripted()
    env = await init_env(ctrl, CapsemSandboxConfig(environment={"FOO": "bar"}))
    assert env.vm_id == "vm-s"
    assert ctrl.started[0]["env"] == {"FOO": "bar"}
    assert ctrl.started[0]["labels"]["managed-by"] == "inspect-capsem"
    assert ctrl.started[0]["labels"]["inspect-capsem-task"] == "t"
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

    ctrl = SlowStop([("test -x", hang)])
    ticks: list[float] = []

    async def ticker() -> None:
        while True:
            ticks.append(time.monotonic())
            await asyncio.sleep(0.01)

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
    assert ctrl.stopped == ["vm-s"]
    assert len(ticks) > 10


async def test_failed_init_teardown_is_bounded(monkeypatch: pytest.MonkeyPatch) -> None:
    def hang(command: str) -> CommandResult:
        raise TimeoutError(command)

    class HungStop(Scripted):
        async def stop_vm(self, vm_id: str) -> None:
            del vm_id
            await asyncio.sleep(10.0)

    monkeypatch.setattr(lifecycle_mod, "_INIT_TEARDOWN_TIMEOUT_SECS", 0.1)
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
    assert started_vms == ["vm-1"]
    assert ctrl.stopped == ["vm-1"]

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


async def test_sample_init_cancelled_or_lost_sdk_create_sweeps_untracked_vm(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """Cancelled or lost-response Hypervisor.create sweeps only the matching per-create nonce VM."""
    from types import SimpleNamespace

    from inspect_capsem._controller import SdkCapsemController

    created_on_server = asyncio.Event()
    deleted: list[str] = []
    mode: dict[str, Any] = {"fail": False, "labels": {}}
    lost_id = "550e8400-e29b-41d4-a716-446655440001"
    sibling_id = "550e8400-e29b-41d4-a716-446655440002"
    named_id = "550e8400-e29b-41d4-a716-446655440003"

    class CancelCreateHyp:
        async def create(self, **kwargs: Any) -> Any:
            mode["labels"] = dict(kwargs.get("labels") or {})
            if mode["fail"]:
                raise RuntimeError("lost HTTP response")
            created_on_server.set()
            await asyncio.sleep(10.0)

        async def list(self) -> Any:
            lbls = mode["labels"]
            sib_lbls = {**lbls, "inspect-capsem-nonce": "concurrent-sibling-nonce"}
            return SimpleNamespace(
                sandboxes=[
                    SimpleNamespace(id=lost_id, name="vm-1", persistent=False, labels=lbls),
                    SimpleNamespace(id=sibling_id, name="vm-2", persistent=False, labels=sib_lbls),
                    SimpleNamespace(id=named_id, name="devbox", persistent=True, labels=lbls),
                ]
            )

        def vm(self, *, id: str) -> Any:
            async def _del() -> None:
                deleted.append(id)

            return SimpleNamespace(delete=_del)

        async def close(self) -> None:
            pass

    monkeypatch.setattr(lifecycle_mod, "_INIT_TEARDOWN_TIMEOUT_SECS", 0.05)
    monkeypatch.setattr(
        sb, "SdkCapsemController", lambda: SdkCapsemController(cast(Any, CancelCreateHyp()))
    )
    task = asyncio.create_task(
        CapsemSandboxEnvironment.sample_init("cancel_lost", CapsemSandboxConfig(), {})
    )
    await asyncio.wait_for(created_on_server.wait(), timeout=2.0)
    task.cancel()
    with pytest.raises(asyncio.CancelledError):
        await task
    assert deleted == [lost_id]

    deleted.clear()
    mode["fail"] = True
    with pytest.raises(RuntimeError, match="lost HTTP response"):
        await CapsemSandboxEnvironment.sample_init("cancel_lost", CapsemSandboxConfig(), {})
    assert deleted == [lost_id]
