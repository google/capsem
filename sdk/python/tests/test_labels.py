"""VM label propagation in the Python SDK."""

from __future__ import annotations

import asyncio
import json
from typing import Any

import pytest
from capsem import VM, Hypervisor
from pydantic import ValidationError

from .facade_gateway import gateway


def test_create_and_fork_propagate_labels_and_expose_them_on_sandbox_info_and_list() -> None:
    async def run() -> None:
        async with gateway() as (url, state), Hypervisor(url, "token") as hv:
            labels = {"suite": "eval", "owner-prefix": "run_1", "k" * 64: "max"}
            vm = await hv.create(labels=labels)
            body = json.loads(state.requests[-1][2])
            assert body["persistent"] is False
            assert body["labels"] == labels
            info = await vm.info()
            assert info.persistent is False
            assert info.labels == labels

            # Fork without labels sends `labels: null` on the wire so the server inherits source labels.
            await vm.fork("inherited")
            fork_inherit_body = json.loads(state.requests[-1][2])
            assert fork_inherit_body["labels"] is None
            assert state.sandbox_labels["forked-inherited"] == labels

            # Fork with explicit labels sends them; fork with `{}` sends `{}` to clear inherited labels.
            await vm.fork("overridden", labels={"suite": "fork"})
            fork_override_body = json.loads(state.requests[-1][2])
            assert fork_override_body["labels"] == {"suite": "fork"}
            assert state.sandbox_labels["forked-overridden"] == {"suite": "fork"}

            await vm.fork("cleared", labels={})
            fork_clear_body = json.loads(state.requests[-1][2])
            assert fork_clear_body["labels"] == {}
            assert state.sandbox_labels["forked-cleared"] is None

            # Empty labels dict on create is sent as `{}` and normalized to `None` by the server.
            empty_vm = await hv.create(labels={})
            empty_body = json.loads(state.requests[-1][2])
            assert empty_body["labels"] == {}
            assert (await empty_vm.info()).labels is None

            state.names = ["vm-a", "vm-b"]
            state.sandbox_labels = {"vm-0": {"suite": "eval"}, "vm-1": None}
            listed = await hv.list()
            assert [s.labels for s in listed.sandboxes] == [{"suite": "eval"}, None]

    asyncio.run(run())


@pytest.mark.parametrize("bad_labels", ["not-a-dict", [("a", "b")], {1: "v"}, {"k": 2}])
def test_invalid_label_types_are_rejected_by_pydantic_before_network(bad_labels: Any) -> None:
    async def run() -> None:
        async with Hypervisor("http://127.0.0.1:1", "token") as hv:
            with pytest.raises(ValidationError):
                await hv.create(labels=bad_labels)
        async with VM("http://127.0.0.1:1", "token", id="vm-0") as vm:
            with pytest.raises(ValidationError):
                await vm.fork("forked", labels=bad_labels)

    asyncio.run(run())
