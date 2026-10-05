"""Existing-session handles borrow the hypervisor's authenticated connection."""

from __future__ import annotations

import asyncio
from typing import Any

import pytest
from capsem import VM, Hypervisor, models
from pydantic import ValidationError

from .facade_gateway import gateway


def test_id_handle_attaches_without_http_and_uses_authenticated_connection() -> None:
    async def run() -> None:
        async with gateway() as (url, state), Hypervisor(url, "token") as hv:
            vm = hv.vm(id="vm-0")
            assert isinstance(vm, VM)
            assert vm.id == "vm-0" and vm.name is None
            assert state.requests == []
            assert isinstance(await vm.info(), models.SandboxInfo)
            assert [(method, path) for method, path, _ in state.requests] == [
                ("GET", "/vms/vm-0/info"),
            ]
            await vm.close()
    asyncio.run(run())


def test_name_handle_resolves_lazily_once_and_retains_canonical_id() -> None:
    async def run() -> None:
        async with gateway() as (url, state), Hypervisor(url, "token") as hv:
            async with hv.vm(name="named") as vm:
                assert vm.id is None and vm.name == "named"
                assert state.requests == []
                await vm.info()
                assert vm.id == "vm-0"
                state.names = ["renamed"]
                await vm.info()
            assert [(method, path) for method, path, _ in state.requests] == [
                ("GET", "/vms/list"), ("GET", "/vms/vm-0/info"),
                ("GET", "/vms/vm-0/info"),
            ]
    asyncio.run(run())


@pytest.mark.parametrize("names", [[], ["named", "named"]])
def test_unresolved_name_handles_never_mutate_sessions(names: list[str]) -> None:
    async def run() -> None:
        async with gateway() as (url, state), Hypervisor(url, "token") as hv:
            state.names = names
            async with hv.vm(name="named") as vm:
                with pytest.raises(LookupError, match="expected one VM"):
                    await vm.delete()
            assert [(method, path) for method, path, _ in state.requests] == [
                ("GET", "/vms/list"),
            ]
    asyncio.run(run())


@pytest.mark.parametrize(
    ("name", "id"), [(None, None), ("", ""), ("named", "vm-0"), ("", None), (None, "")],
)
def test_handle_requires_exactly_one_nonempty_selector(name: str | None, id: str | None) -> None:
    async def run() -> None:
        async with gateway() as (url, state), Hypervisor(url, "token") as hv:
            with pytest.raises(ValueError, match="exactly one"):
                hv.vm(name=name, id=id)
            assert state.requests == []
    asyncio.run(run())


@pytest.mark.parametrize("selector", [{"name": 1}, {"id": True}, {"name": b"named"}])
def test_handle_reuses_strict_selector_validation(selector: dict[str, Any]) -> None:
    async def run() -> None:
        async with gateway() as (url, state), Hypervisor(url, "token") as hv:
            with pytest.raises(ValidationError):
                hv.vm(**selector)
            assert state.requests == []
    asyncio.run(run())


def test_closing_handle_preserves_parent_and_siblings_without_stopping_session() -> None:
    async def run() -> None:
        async with gateway() as (url, state), Hypervisor(url, "token") as hv:
            first = hv.vm(id="vm-0")
            sibling = hv.vm(id="vm-1")
            await first.info()
            await first.close()
            await first.close()
            with pytest.raises(RuntimeError, match="closed"):
                await first.info()
            with pytest.raises(RuntimeError, match="closed"):
                async with first:
                    pass
            assert isinstance(await hv.list(), models.ListResponse)
            async with sibling:
                assert isinstance(await sibling.info(), models.SandboxInfo)
            assert [(method, path) for method, path, _ in state.requests] == [
                ("GET", "/vms/vm-0/info"), ("GET", "/vms/list"),
                ("GET", "/vms/vm-1/info"),
            ]
    asyncio.run(run())


def test_parent_close_ends_shared_transport_without_stopping_attached_sessions() -> None:
    async def run() -> None:
        async with gateway() as (url, state):
            hv = Hypervisor(url, "token")
            vm = hv.vm(id="vm-0")
            await vm.info()
            await hv.close()
            await hv.close()
            with pytest.raises(RuntimeError, match="closed"):
                await vm.info()
            with pytest.raises(RuntimeError, match="closed"):
                hv.vm(id="vm-1")
            await vm.close()
            assert [(method, path) for method, path, _ in state.requests] == [
                ("GET", "/vms/vm-0/info"),
            ]
    asyncio.run(run())
