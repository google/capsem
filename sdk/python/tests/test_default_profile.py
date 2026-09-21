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
            state.default_vm_profile_id = "co-work"
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
            state.default_vm_profile_id = None
            with pytest.raises(RuntimeError, match="names no default vm profile"):
                await hv.create()
            assert "/vms/create" not in [path for _, path, _ in state.requests]
    asyncio.run(run())


def test_a_container_takes_the_catalog_container_default_not_the_vm_one() -> None:
    """The two defaults are free to differ; a container must not take the VM's."""
    async def run() -> None:
        async with gateway() as (url, state), Hypervisor(url, "token") as hv:
            state.default_vm_profile_id = "code"
            state.default_container_profile_id = "co-work"
            await hv.create(image="alpine:3")
            await hv.create()
            bodies = [json.loads(body) for _, path, body in state.requests if path == "/vms/create"]
            assert [body["profile_id"] for body in bodies] == ["co-work", "code"]
    asyncio.run(run())


def test_a_catalog_without_a_container_default_names_the_runtime_it_lacks() -> None:
    async def run() -> None:
        async with gateway() as (url, state), Hypervisor(url, "token") as hv:
            state.default_container_profile_id = None
            with pytest.raises(RuntimeError, match="names no default container profile"):
                await hv.create(image="alpine:3")
            assert "/vms/create" not in [path for _, path, _ in state.requests]
            # The VM default is untouched by the container's absence.
            await hv.create()
    asyncio.run(run())
