"""Saved revision selectors cross the real facade HTTP boundary unchanged."""

from __future__ import annotations

import asyncio
import json

import pytest
from capsem import Hypervisor, models

from .facade_gateway import gateway


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
