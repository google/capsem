"""Public clients must preserve identity, resources and connection lifetimes."""

from __future__ import annotations

import asyncio
import json

import pytest
from capsem import VM, HttpError, Hypervisor, models

from .facade_gateway import gateway


def test_hypervisor_creation_defaults_and_connection_ownership() -> None:
    async def run() -> None:
        async with gateway() as (url, state), Hypervisor(url, "token") as hv:
            assert isinstance(await hv.info(), models.HypervisorInfo)
            assert isinstance(await hv.list(), models.ListResponse)
            vm = await hv.create("code", name="new", vcpu=4, memory="8G", env={"LANG": "C"})
            assert vm.id == "created-id" and vm.name == "new"
            body = json.loads(state.requests[-1][2])
            assert body == {"profile_id": "code", "name": "new", "persistent": True,
                            "cpus": 4, "ram_mb": 8192, "env": {"LANG": "C"}}
            async with vm:
                assert isinstance(await vm.info(), models.SandboxInfo)
            with pytest.raises(RuntimeError, match="closed"):
                await vm.info()
            with pytest.raises(RuntimeError, match="closed"):
                async with vm:
                    pass
            temporary = await hv.create("code")
            body = json.loads(state.requests[-1][2])
            assert body["persistent"] is False and body["name"] is None
            assert body["cpus"] is None and body["ram_mb"] is None
            assert isinstance(await hv.log(models.HostLogSource.SERVICE, grep="boot", tail=3, max_bytes=1024), models.HostLogsResponse)
            assert isinstance(await hv.update(), models.UpdateActionResponse)
            assert json.loads(state.requests[-1][2]) == {"confirmed": True}
        await hv.close()
        with pytest.raises(RuntimeError, match="closed"):
            await temporary.info()
    asyncio.run(run())


def test_name_is_resolved_once_and_each_vm_interface_returns_typed_results() -> None:
    async def run() -> None:
        async with gateway() as (url, state), VM(url, "token", name="named") as vm:
            assert vm.id is None and vm.name == "named"
            assert isinstance(await vm.info(), models.SandboxInfo)
            assert vm.id == "vm-0"
            state.names = ["renamed"]
            assert isinstance(await vm.exec("echo hello", timeout_secs=12), models.ExecResponse)
            assert json.loads(state.requests[-1][2]) == {"command": "echo hello", "timeout_secs": 12}
            assert isinstance(await vm.start(), models.ProvisionResponse)
            assert isinstance(await vm.pause(), models.VmActionResponse)
            assert isinstance(await vm.resume(), models.ProvisionResponse)
            assert isinstance(await vm.log(grep="ready", tail=5, max_bytes=2048), models.LogsResponse)
            assert isinstance(await vm.history(layer=models.HistoryLayerFilter.EXEC, search="hello", limit=2, offset=1), models.HistoryResponse)
            assert isinstance(await vm.timeline(layers=[models.TimelineLayer.EXEC, models.TimelineLayer.MODEL], since="now", limit=3), models.TimelineResponse)
            assert "layers=exec%2Cmodel" in state.requests[-1][1]
            await vm.timeline()
            assert isinstance(await vm.list("/work", depth=2), models.FileListResponse)
            assert isinstance(await vm.changes("cp-10", limit=3, offset=1), models.ChangesResponse)
            assert isinstance(await vm.stats.summary(), models.VmStatsSummaryResponse)
            assert isinstance(await vm.stats.details(), models.VmStatsDetailResponse)
            assert isinstance(await vm.snapshots.list(), models.SnapshotsList)
            assert isinstance(await vm.snapshots.status(), models.SnapshotsStatus)
            assert isinstance(await vm.copy.to_vm("/work/bytes", b"\x00\xff"), models.UploadResponse)
            assert await vm.copy.from_vm("/work/bytes") == b"\x00\xff"
            with pytest.raises(HttpError) as error:
                await vm.copy.from_vm("/missing")
            assert error.value.status == 404
            child = await vm.fork("child", description="copy")
            assert child.id == "forked-id" and child.name == "child"
            await child.close()
            assert isinstance(await vm.stop(), models.StopResponse)
            assert isinstance(await vm.delete(), models.VmActionResponse)
            assert sum(path == "/vms/list" for _, path, _ in state.requests) == 1
    asyncio.run(run())


@pytest.mark.parametrize("names", [[], ["named", "named"]])
def test_missing_and_ambiguous_names_never_mutate_a_vm(names: list[str]) -> None:
    async def run() -> None:
        async with gateway() as (url, state), VM(url, "token", name="named") as vm:
            state.names = names
            with pytest.raises(LookupError, match="expected one VM"):
                await vm.delete()
            assert [(method, path) for method, path, _ in state.requests] == [("GET", "/vms/list")]
    asyncio.run(run())


@pytest.mark.parametrize(("name", "id"), [(None, None), ("", ""), ("name", "id")])
def test_vm_requires_one_selector(name: str | None, id: str | None) -> None:
    with pytest.raises(ValueError, match="exactly one"):
        VM("http://127.0.0.1:1", "token", name=name, id=id)


def test_cancelling_execution_does_not_retry_or_break_the_connection() -> None:
    async def run() -> None:
        async with gateway() as (url, state), VM(url, "token", id="vm-0") as vm:
            state.wait_for_exec = True
            task = asyncio.create_task(vm.exec("waiting"))
            await asyncio.wait_for(state.exec_entered.wait(), timeout=1)
            task.cancel()
            with pytest.raises(asyncio.CancelledError):
                await task
            state.exec_release.set()
            assert isinstance(await vm.info(), models.SandboxInfo)
            assert sum(path.endswith("/exec") for _, path, _ in state.requests) == 1
    asyncio.run(run())


def test_http_deadline_bounds_execution_without_replaying_it() -> None:
    async def run() -> None:
        async with gateway() as (url, state), VM(url, "token", id="vm-0", timeout=0.05) as vm:
            state.wait_for_exec = True
            with pytest.raises(TimeoutError):
                await vm.exec("waiting")
            assert state.exec_entered.is_set()
            assert len(state.requests) == 1
    asyncio.run(run())


@pytest.mark.parametrize("memory", [0, -1, True, "0G", "8GB", "junk"])
def test_invalid_memory_is_rejected_before_network(memory: str | int) -> None:
    async def run() -> None:
        async with Hypervisor("http://127.0.0.1:1", "token") as hv:
            with pytest.raises(ValueError, match="memory"):
                await hv.create("code", memory=memory)
    asyncio.run(run())


@pytest.mark.parametrize(("memory", "expected"), [(512, 512), ("256M", 256), ("2g", 2048)])
def test_memory_units_are_converted(memory: str | int, expected: int) -> None:
    async def run() -> None:
        async with gateway() as (url, state), Hypervisor(url, "token") as hv:
            await hv.create("code", memory=memory)
            assert json.loads(state.requests[-1][2])["ram_mb"] == expected
    asyncio.run(run())


def test_zero_vcpu_is_rejected_before_network() -> None:
    async def run() -> None:
        async with Hypervisor("http://127.0.0.1:1", "token") as hv:
            with pytest.raises(ValueError, match="vcpu"):
                await hv.create("code", vcpu=0)
    asyncio.run(run())
