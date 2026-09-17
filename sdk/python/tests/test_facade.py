"""Public clients must preserve identity, resources and connection lifetimes."""

from __future__ import annotations

import asyncio
import json
from typing import Any

import pytest
from capsem import (
    VM,
    ExecResult,
    HttpError,
    Hypervisor,
    Port,
    Registry,
    models,
)

from .facade_gateway import gateway


def test_exec_result_prints_stdout_and_preserves_exact_bytes() -> None:
    result = ExecResult(
        exit_code=0,
        stdout=models.ExecOutput(encoding=models.ExecOutputEncoding.UTF8, data="café\n"),
        stderr=models.ExecOutput(encoding=models.ExecOutputEncoding.BASE64, data="AP8K"),
    )
    assert str(result) == "café"
    assert result.stdout_bytes == "café\n".encode()
    assert result.stderr_bytes == bytes([0, 0xFF, 10])


def test_hypervisor_creation_defaults_and_connection_ownership() -> None:
    async def run() -> None:
        async with gateway() as (url, state), Hypervisor(url, "token") as hv:
            assert isinstance(await hv.info(), models.HypervisorInfo)
            assert isinstance(await hv.list(), models.ListResponse)
            profiles = await hv.profiles.list()
            assert profiles and isinstance(profiles[0], models.ProfileSummary)
            network = await hv.networks.create("team")
            vm = await hv.create(
                profile=profiles[0], name="new", cpus=4, memory=8,
                env={"LANG": "C"}, networks=[network],
            )
            assert vm.id == "created-id" and vm.name == "new"
            body = json.loads(state.requests[-1][2])
            assert body == {"profile_id": "code", "name": "new", "persistent": True,
                            "cpus": 4, "ram_mb": 8192, "env": {"LANG": "C"}, "networks": ["team"]}
            async with vm:
                assert isinstance(await vm.info(), models.SandboxInfo)
            with pytest.raises(RuntimeError, match="closed"):
                await vm.info()
            with pytest.raises(RuntimeError, match="closed"):
                async with vm:
                    pass
            temporary = await hv.create()
            body = json.loads(state.requests[-1][2])
            assert body["persistent"] is False and body["name"] is None
            assert body["cpus"] is None and body["ram_mb"] is None
            assert isinstance(await hv.log(models.HostLogSource.SERVICE, grep="boot", tail=3, max_bytes=1024), models.HostLogsResponse)
            assert isinstance(await hv.run("printf ok", timeout_secs=4), ExecResult)
            assert isinstance(await hv.debug.panics(since="5m", limit=3), models.PanicsResponse)
            assert isinstance(await hv.debug.triage(vm_id="vm-0", since="1h", limit=2), models.TriageResponse)
            assert isinstance(await hv.purge(all=True), models.PurgeResponse)
            mcp = hv.profiles.mcp("code")
            assert isinstance(await mcp.info(), models.ProfileMcpInfoResponse)
            assert isinstance(await mcp.servers(), list)
            assert isinstance(await mcp.default_permission(), models.McpDefaultPermissionResponse)
            assert isinstance(await mcp.tools("local"), list)
            assert isinstance(await mcp.refresh("local"), models.McpRefreshResponse)
            assert await mcp.call("local", "read_file", {"path": "/tmp/x"}) is not None
            assert isinstance(await hv.update(), models.UpdateActionResponse)
            assert json.loads(state.requests[-1][2]) == {"confirmed": True}
            restarted = await hv.restart()
            assert restarted.status is models.RestartStatus.ACCEPTED
            assert restarted.authentication is models.RestartAuthentication.NEW_TOKEN_REQUIRED
            assert state.requests[-1] == ("POST", "/restart", b"")
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
            assert isinstance(await vm.exec("echo hello", timeout_secs=12), ExecResult)
            assert json.loads(state.requests[-1][2]) == {"command": "echo hello", "timeout_secs": 12}
            assert isinstance(await vm.start(), models.ProvisionResponse)
            assert isinstance(await vm.persist("saved"), models.PersistResponse)
            assert isinstance(await vm.pause(), models.VmActionResponse)
            assert isinstance(await vm.resume(), models.ProvisionResponse)
            assert isinstance(await vm.log(grep="ready", tail=5, max_bytes=2048), models.LogsResponse)
            assert isinstance(await vm.history(layer=models.HistoryLayerFilter.EXEC, search="hello", limit=2, offset=1), models.HistoryResponse)
            assert isinstance(await vm.timeline(layers=[models.TimelineLayer.EXEC, models.TimelineLayer.MODEL], since="now", limit=3), models.TimelineResponse)
            assert "layers=exec%2Cmodel" in state.requests[-1][1]
            await vm.timeline()
            assert isinstance(await vm.files.list("/work", depth=2), models.FileListResponse)
            await vm.files.list()
            assert state.requests[-1][1] == "/vms/vm-0/files/list"
            assert isinstance(await vm.files.history("cp-10", limit=3, offset=1), models.ChangesResponse)
            assert isinstance(await vm.stats.summary(), models.VmStatsSummaryResponse)
            assert isinstance(await vm.stats.details(), models.VmStatsDetailResponse)
            assert isinstance(await vm.snapshots.list(), models.SnapshotsList)
            assert isinstance(await vm.snapshots.status(), models.SnapshotsStatus)
            assert isinstance(await vm.files.write("/work/bytes", b"\x00\xff"), models.UploadResponse)
            assert await vm.files.read("/work/bytes") == b"\x00\xff"
            with pytest.raises(HttpError) as error:
                await vm.files.read("/missing")
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


def test_container_create_is_ready_and_status_is_read_only() -> None:
    async def run() -> None:
        async with gateway() as (url, state), Hypervisor(url, "token") as hv:
            registry = Registry(username="robot", password="registry-secret")
            assert "registry-secret" not in repr(registry)
            vm = await hv.create(
                image="docker://busybox:latest", command=[], env={"MODE": "preview"}, registry=registry,
            )
            body = json.loads(state.requests[-1][2])
            assert body["env"] is None
            assert body["container"] == {
                "image": "docker://busybox:latest", "args": [], "env": {"MODE": "preview"},
                "registry": {"username": "robot", "password": "registry-secret", "ca_pem": None},
                "attach": False,
            }
            assert (await vm.container.status()).image == "docker://busybox:latest"
            assert {method for method, path, _ in state.requests if path.endswith("/container")} == {"GET"}
    asyncio.run(run())


def test_container_options_without_an_image_are_rejected_before_http() -> None:
    async def run() -> None:
        async with gateway() as (url, _), Hypervisor(url, "token") as hv:
            with pytest.raises(ValueError, match="image"):
                await hv.create(command=["true"])
            invalid_registry: Any = models.RegistryAccess()
            with pytest.raises(TypeError, match="Registry"):
                await hv.create(image="busybox", registry=invalid_registry)
    asyncio.run(run())


def test_ports_hide_wire_exposures_and_infer_the_container_target() -> None:
    async def run() -> None:
        async with gateway() as (url, state), Hypervisor(url, "token") as hv:
            vm = await hv.create(image="nginx:alpine")
            plain = await vm.ports.open(8080)
            assert isinstance(plain, Port)
            assert plain.guest == 8080 and plain.authenticate is False
            authenticated = await vm.ports.open(3000, authenticate=True)
            assert authenticated.authenticate is True
            assert authenticated.url is not None and authenticated.bootstrap_token is not None
            assert await vm.ports.list() == []
            request_count = len(state.requests)
            raw_id: Any = plain.id
            with pytest.raises(TypeError, match="port must be an object"):
                await vm.ports.close(raw_id)
            assert len(state.requests) == request_count
            assert isinstance(await vm.ports.close(plain), models.VmActionResponse)
            create_bodies = [json.loads(body) for method, path, body in state.requests
                             if method == "POST" and path.endswith("/exposures")]
            assert create_bodies == [
                {"guest_port": 8080, "host_port": 0, "target": "container", "access": "loopback_tcp"},
                {"guest_port": 3000, "host_port": 0, "target": "container", "access": "http_preview"},
            ]
            assert [(method, path.split("?")[0]) for method, path, _ in state.requests] == [
                ("POST", "/vms/create"),
                ("POST", "/vms/created-id/exposures"),
                ("POST", "/vms/created-id/exposures"),
                ("POST", f"/vms/created-id/exposures/{authenticated.id}/preview-session"),
                ("GET", "/vms/created-id/exposures"),
                ("DELETE", f"/vms/created-id/exposures/{plain.id}"),
            ]
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


@pytest.mark.parametrize("memory", [0, -1, True, 1.5, "8G"])
def test_invalid_memory_is_rejected_before_network(memory: Any) -> None:
    async def run() -> None:
        async with Hypervisor("http://127.0.0.1:1", "token") as hv:
            with pytest.raises(ValueError, match="memory"):
                await hv.create(memory=memory)
    asyncio.run(run())


@pytest.mark.parametrize(("memory", "expected"), [(1, 1024), (8, 8192)])
def test_memory_is_measured_in_gibibytes(memory: int, expected: int) -> None:
    async def run() -> None:
        async with gateway() as (url, state), Hypervisor(url, "token") as hv:
            await hv.create(memory=memory)
            assert json.loads(state.requests[-1][2])["ram_mb"] == expected
    asyncio.run(run())


def test_zero_cpus_is_rejected_before_network() -> None:
    async def run() -> None:
        async with Hypervisor("http://127.0.0.1:1", "token") as hv:
            with pytest.raises(ValueError, match="cpus"):
                await hv.create(cpus=0)
    asyncio.run(run())


def test_network_resource_maps_typed_lifecycle_and_cursor_logs() -> None:
    async def run() -> None:
        async with gateway() as (url, state), Hypervisor(url, "token") as hv:
            created = await hv.networks.create("team")
            assert [network.id for network in await hv.networks.list()] == ["net-1"]
            await hv.networks.inspect(created.id)
            async with VM(url, "token", id="vm-0") as vm:
                assert [network.id for network in await vm.networks.list()] == ["net-1"]
                await vm.networks.attach(created)
                await vm.networks.detach(created)
            raw_id: Any = created.id
            request_count = len(state.requests)
            with pytest.raises(TypeError, match="network must be an object"):
                await hv.networks.logs(raw_id)
            with pytest.raises(TypeError, match="network must be an object"):
                await hv.networks.delete(raw_id)
            assert len(state.requests) == request_count
            await hv.networks.logs(created, cursor="next", limit=4, event_type="network.connect")
            await hv.networks.delete(created)
            assert [(method, path.split("?")[0]) for method, path, _ in state.requests] == [
                ("POST", "/networks"),
                ("GET", "/networks"),
                ("GET", f"/networks/{created.id}"),
                ("GET", "/networks"),
                ("PUT", f"/networks/{created.id}/members/vm-0"),
                ("DELETE", f"/networks/{created.id}/members/vm-0"),
                ("GET", f"/networks/{created.id}/logs"),
                ("DELETE", f"/networks/{created.id}"),
            ]
            assert "cursor=next" in state.requests[-2][1]
            assert "type=network.connect" in state.requests[-2][1]
    asyncio.run(run())
