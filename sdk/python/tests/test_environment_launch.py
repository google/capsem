"""Saved revision selectors cross the real facade HTTP boundary unchanged."""

from __future__ import annotations

import asyncio
import json

import pytest
from capsem import Hypervisor, models

from .facade_gateway import gateway
from .test_deadlines import RETAINED_SELECTION


def test_environment_launch_preserves_revision_and_refuses_inline_overrides():
    async def run():
        reference = models.EnvironmentRevisionReference(
            environment_id="00000000-0000-4000-8000-000000000001",
            revision_id="00000000-0000-4000-8000-000000000002",
        )
        async with gateway() as (url, state), Hypervisor(url, "token") as hv:
            vm = await hv.create(environment=reference)
            assert vm.id == "created-id"
            assert json.loads(state.requests[-1][2])["environment"] == reference.model_dump()
            assert "attach" not in json.loads(state.requests[-1][2])
            assert state.requests[0][1].endswith('/readiness')
            attached = await hv.create(environment=reference, attach=True)
            body = json.loads(state.requests[-1][2])
            assert body["environment"] == reference.model_dump() and body["attach"] is True
            assert "container" not in body and body.get("env") is None
            count = len(state.requests)
            with pytest.raises(ValueError):
                await hv.create(attach=True)
            with pytest.raises(ValueError):
                await hv.create(environment=reference, env={})
            with pytest.raises(ValueError):
                await hv.create(environment=reference, image="busybox")
            assert len(state.requests) == count
            await vm.close()
            await attached.close()

    asyncio.run(run())


@pytest.mark.parametrize("override_destination", [False, True])
def test_clone_keeps_the_original_source_and_explicit_destination(override_destination):
    async def run():
        reference = models.EnvironmentRevisionReference(
            environment_id="00000000-0000-4000-8000-000000000001",
            revision_id="00000000-0000-4000-8000-000000000004",
        )
        async with gateway() as (url, state), Hypervisor(url, "token") as hv:
            state.vm_readiness["source-name"] = {
                "selection": RETAINED_SELECTION, "startup_timeout_seconds": 600,
            }
            vm = await hv.create(name="clone", from_vm="source-name",
                                 environment=reference if override_destination else None)
            assert vm.id == "created-id"
            expected_paths = ["/vms/source-name/readiness?intent=clone"]
            if override_destination:
                expected_paths.append(
                    f"/environments/{reference.environment_id}/revisions/{reference.revision_id}/readiness"
                )
            assert [path for _, path, _ in state.requests] == [*expected_paths, "/vms/create"]
            body = json.loads(state.requests[-1][2])
            assert body["from"] == "source-name"
            assert body["expected_source"] == RETAINED_SELECTION
            if override_destination:
                assert body["environment"] == reference.model_dump()
            else:
                assert "environment" not in body
            await vm.close()
    asyncio.run(run())
