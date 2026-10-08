"""Credential injection is one authenticated request and returns no material."""
import asyncio

import pytest
from capsem import HttpError, Hypervisor

from .test_images import gateway


def test_injection_file_and_memory_preserve_exact_host_request_and_reference():
    async def run():
        reference = "credential:blake3:" + "a" * 64
        for storage in ["file", "memory"]:
            async with gateway(response={"credential_ref": reference, "storage": storage}) as (url, received):
                async with Hypervisor(url, "gateway-token") as hv:
                    response = await hv.credentials.inject("openai", "private-test-key", storage=storage)
                    assert response.credential_ref == reference
                    assert response.storage == storage
                    assert "private-test-key" not in repr(response)
                assert received == [("POST", "/credentials/inject", {
                    "provider": "openai", "value": "private-test-key", "storage": storage,
                })]
    asyncio.run(run())


def test_injection_refusal_is_not_replayed():
    async def run():
        async with gateway(response={"error": "credential handoff unavailable"}, status=503) as (url, received), Hypervisor(url, "gateway-token") as hv:
            with pytest.raises(HttpError):
                await hv.credentials.inject("google", "private-test-key", storage="memory")
            assert len(received) == 1
    asyncio.run(run())
