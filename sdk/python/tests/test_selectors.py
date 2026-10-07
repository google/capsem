"""A supplied selector must never disappear through Python truthiness."""

from __future__ import annotations

import asyncio
from typing import Any

import pytest
from capsem import VM, Hypervisor

from .facade_gateway import gateway


@pytest.mark.parametrize("empty", ["", False, 0, [], {}])
@pytest.mark.parametrize("selected", [{"id": "vm-0"}, {"name": "named"}])
def test_supplied_second_selector_is_rejected_before_http(
    selected: dict[str, str], empty: Any,
) -> None:
    selectors: dict[str, Any] = {**selected, "name" if "id" in selected else "id": empty}

    async def run() -> None:
        async with gateway() as (url, state), Hypervisor(url, "token") as hv:
            with pytest.raises(ValueError):
                VM(url, "token", **selectors)
            with pytest.raises(ValueError):
                hv.vm(**selectors)
            assert state.requests == []
            await hv.list()
            assert [(method, path) for method, path, _ in state.requests] == [("GET", "/vms/list")]

    asyncio.run(run())


@pytest.mark.parametrize("key", ["id", "name"])
@pytest.mark.parametrize("invalid", ["", False, 0, [], {}, 12, True])
def test_single_selector_must_be_a_nonempty_string(key: str, invalid: Any) -> None:
    async def run() -> None:
        async with gateway() as (url, state), Hypervisor(url, "token") as hv:
            with pytest.raises(ValueError):
                VM(url, "token", **{key: invalid})
            with pytest.raises(ValueError):
                hv.vm(**{key: invalid})
            assert state.requests == []

    asyncio.run(run())


@pytest.mark.parametrize("selectors", [{"id": "vm-0", "name": None}, {"id": None, "name": "named"}])
def test_explicit_none_keeps_valid_attachment_and_shared_connection(selectors: dict[str, str | None]) -> None:
    async def run() -> None:
        async with gateway() as (url, state), Hypervisor(url, "token") as hv:
            attached = hv.vm(**selectors)
            assert state.requests == []
            await attached.info()
            assert attached.id == "vm-0"
            await attached.close()
            with pytest.raises(RuntimeError, match="closed"):
                await attached.info()
            sibling = hv.vm(id="vm-0")
            await sibling.info()
            paths = [path for _, path, _ in state.requests]
            assert paths == (["/vms/list"] if selectors["name"] else []) + ["/vms/vm-0/info"] * 2
            assert all(method == "GET" for method, _, _ in state.requests)
            await hv.close()
            with pytest.raises(RuntimeError, match="closed"):
                await sibling.info()
            assert len(state.requests) == len(paths)
            await sibling.close()

    asyncio.run(run())
