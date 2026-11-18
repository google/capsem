"""Environment facades preserve identity, decisions and explicit reconciliation."""
from __future__ import annotations

import asyncio

import pytest
from capsem import HttpError, Hypervisor, models

from .test_images import gateway

REQUEST = "00000000-0000-4000-8000-000000000001"
ENVIRONMENT = "00000000-0000-4000-8000-000000000002"
PENDING: dict[str, object] = {"request_id": REQUEST, "environment_id": ENVIRONMENT, "decision": "approved",
           "application": "pending", "result": None, "application_error": None}
CURRENT: dict[str, object] = {"document": {"identity": {"schema_version": 1, "id": ENVIRONMENT, "revision_id": REQUEST},
                        "specification": {"name": "development"}},
           "timestamp_unix_ms": 1, "actor": "operator", "reason": "approved", "request_id": REQUEST}


def request():
    return models.EnvironmentCreateRequest.model_validate({
        "request_id": REQUEST, "specification": {"name": "development"}, "reason": "prepare blueprint",
    })


def test_environment_creation_inspection_and_explicit_reconciliation():
    async def run():
        response = dict(PENDING)
        async with gateway(response=response) as (url, received):
            async with Hypervisor(url, "gateway-token") as hv:
                created = await hv.environments.create(request())
                assert created.request_id == REQUEST
                assert created.decision == models.RequestState.APPROVED
                assert created.application == models.RequestApplicationState.PENDING
                assert created.result is None
                response.clear()
                response.update(CURRENT)
                current = await hv.environments.info(ENVIRONMENT)
                assert current.document.identity.id == ENVIRONMENT
                assert current.document.specification.name == "development"
                assert current.actor == "operator"
                response.clear()
                response.update(PENDING)
                assert await hv.environments.create(request()) == created
                environments = hv.environments
            assert [(method, path) for method, path, _ in received] == [
                ("POST", "/environments/create"), ("GET", f"/environments/{ENVIRONMENT}/info"),
                ("POST", "/environments/create"),
            ]
            for _, _, body in received[::2]:
                assert body["request_id"] == REQUEST
                assert body["specification"]["name"] == "development"
                assert "actor" not in body
            with pytest.raises(RuntimeError, match="closed"):
                await environments.create(request())
            assert len(received) == 3
    asyncio.run(run())


def test_environment_conflict_is_preserved_without_replay_or_refresh():
    async def run():
        async with gateway(response={"error": "changed intent"}, status=409) as (url, received), Hypervisor(url, "gateway-token") as hv:
            with pytest.raises(HttpError) as refused:
                await hv.environments.create(request())
            assert refused.value.status == 409
            assert len(received) == 1
    asyncio.run(run())


def test_environment_creation_refuses_substituted_request_identity():
    async def run():
        async with gateway(response={**PENDING, "request_id": ENVIRONMENT}) as (url, received), Hypervisor(url, "gateway-token") as hv:
            with pytest.raises(ValueError, match="identity changed"):
                await hv.environments.create(request())
            assert len(received) == 1
    asyncio.run(run())


def test_environment_inspection_refuses_substituted_environment_identity():
    async def run():
        async with gateway(response=CURRENT) as (url, received), Hypervisor(url, "gateway-token") as hv:
            with pytest.raises(ValueError, match="identity changed"):
                await hv.environments.info(REQUEST)
            assert len(received) == 1
    asyncio.run(run())


def test_environment_creation_preserves_explicit_deadline_and_cancellation():
    async def run():
        async with gateway(response=PENDING, response_delay=0.15) as (url, received), Hypervisor(url, "gateway-token", timeout=0.03) as hv:
            assert (await hv.environments.create(request())).request_id == REQUEST
            with pytest.raises(TimeoutError):
                await hv.environments.create(request(), request_timeout=0.03)
            with pytest.raises(TimeoutError):
                async with asyncio.timeout(0.03):
                    await hv.environments.create(request())
            assert [path for _, path, _ in received] == ["/environments/create"] * 3
    asyncio.run(run())
