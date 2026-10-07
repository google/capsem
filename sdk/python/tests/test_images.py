"""Friendly image APIs use the typed gateway and own no registry state."""

from __future__ import annotations

import asyncio
import json
from contextlib import asynccontextmanager

import pytest
from aiohttp import web
from capsem import HttpError, Hypervisor, Registry, models
from pydantic import ValidationError

PIN = "registry.example/code@sha256:" + "a" * 64
CATALOG = {
    "catalog": {
        "reference": "registry.example/catalog:nightly",
        "digest": "sha256:" + "c" * 64,
        "channel": "nightly",
    },
    "images": [
        {
            "name": "code",
            "description": "Tools",
            "architectures": ["amd64"],
            "image": PIN,
            "cached": "unknown",
        },
        {
            "name": "other",
            "description": "Other runtime",
            "architectures": ["arm64"],
            "image": None,
            "cached": "unknown",
        },
    ],
}
PULL = {"image": "code", "resolved": PIN, "digest": "sha256:" + "b" * 64}


@asynccontextmanager
async def gateway(*, response=None, status=200):
    received = []

    async def handle(request):
        assert request.headers["Authorization"] == "Bearer gateway-token"
        body = await request.text()
        received.append(
            (request.method, request.path_qs, json.loads(body) if body else None)
        )
        return web.json_response(
            response
            if response is not None
            else (CATALOG if request.path == "/images" else PULL),
            status=status,
        )

    app = web.Application()
    app.router.add_route("*", "/{path:.*}", handle)
    runner = web.AppRunner(app)
    await runner.setup()
    site = web.TCPSite(runner, "127.0.0.1", 0)
    await site.start()
    try:
        address = runner.addresses[0]
        yield f"http://127.0.0.1:{address[1]}", received
    finally:
        await runner.cleanup()


def test_images_preserve_catalog_and_per_call_registry_access():
    async def run():
        async with gateway() as (url, received), Hypervisor(url, "gateway-token") as hv:
            catalog = await hv.images.list(refresh=True)
            assert isinstance(catalog, models.ImageListResponse)
            assert catalog.catalog is not None and catalog.catalog.channel == "nightly"
            assert catalog.images[0].image == PIN
            assert catalog.images[1].image is None
            assert catalog.images[0].cached == models.ImageCacheState.UNKNOWN
            registry = Registry(username="robot", password="private", ca_pem="CA")
            pulled = await hv.images.pull("code", registry=registry)
            assert isinstance(pulled, models.ImagePullResponse)
            assert pulled.resolved == PIN
            await hv.images.pull("code")
            assert received == [
                ("GET", "/images?refresh=true", None),
                (
                    "POST",
                    "/images/pull",
                    {
                        "image": "code",
                        "registry": {
                            "username": "robot",
                            "password": "private",
                            "ca_pem": "CA",
                        },
                    },
                ),
                ("POST", "/images/pull", {"image": "code"}),
            ]
            images = hv.images
        count = len(received)
        with pytest.raises(RuntimeError, match="closed"):
            await images.list()
        with pytest.raises(RuntimeError, match="closed"):
            await images.pull("code")
        assert len(received) == count

    asyncio.run(run())


def test_invalid_image_inputs_never_reach_the_gateway():
    async def run():
        async with gateway() as (url, received), Hypervisor(url, "gateway-token") as hv:
            for image in ["", "  ", 1, None]:
                with pytest.raises(ValueError, match="nonempty"):
                    await hv.images.pull(image)
            with pytest.raises(TypeError, match="Registry"):
                await hv.images.pull("code", registry={"password": "private"})
            with pytest.raises(ValidationError):
                await hv.images.list(refresh=1)
            assert received == []

    asyncio.run(run())


@pytest.mark.parametrize(
    "response",
    [
        {"images": [{"name": "code"}]},
        {"images": [{**CATALOG["images"][0], "cached": "ready"}]},
    ],
)
def test_catalog_does_not_invent_compatibility_or_cache_authority(response):
    async def run():
        async with (
            gateway(response=response) as (url, received),
            Hypervisor(url, "gateway-token") as hv,
        ):
            with pytest.raises(ValidationError):
                await hv.images.list()
            assert len(received) == 1

    asyncio.run(run())


def test_pull_preserves_refusal_without_retry():
    async def run():
        async with (
            gateway(response={"error": "image refused"}, status=403) as (url, received),
            Hypervisor(url, "gateway-token") as hv,
        ):
            with pytest.raises(HttpError) as refused:
                await hv.images.pull("code")
            assert refused.value.status == 403
            assert "image refused" in str(refused.value)
            assert len(received) == 1

    asyncio.run(run())
