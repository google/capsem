"""The gateway names the default profile; clients must not compile one in."""

from __future__ import annotations

import asyncio
import json

import pytest
from capsem import Hypervisor

from .facade_gateway import gateway


def test_create_and_run_use_the_catalog_default_and_resolve_it_once() -> None:
    async def run() -> None:
        async with gateway() as (url, state), Hypervisor(url, "token") as hv:
            state.default_profile_id = "co-work"
            await hv.create()
            await hv.run("true")
            bodies = [json.loads(body) for _, path, body in state.requests if path in ("/vms/create", "/run")]
            assert [body["profile_id"] for body in bodies] == ["co-work", "co-work"]
            assert [path for _, path, _ in state.requests].count("/status") == 1
    asyncio.run(run())


def test_a_named_profile_never_asks_the_gateway_for_the_default() -> None:
    async def run() -> None:
        async with gateway() as (url, state), Hypervisor(url, "token") as hv:
            profile = (await hv.profiles.list())[0]
            await hv.create(profile=profile)
            assert "/status" not in [path for _, path, _ in state.requests]
    asyncio.run(run())


def test_a_catalog_without_a_default_fails_with_a_usable_message() -> None:
    async def run() -> None:
        async with gateway() as (url, state), Hypervisor(url, "token") as hv:
            state.default_profile_id = None
            with pytest.raises(RuntimeError, match="names no default profile"):
                await hv.create()
            assert "/vms/create" not in [path for _, path, _ in state.requests]
    asyncio.run(run())
