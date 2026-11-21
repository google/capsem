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


def test_request_facade_preserves_review_inputs_and_correlates_identity():
    async def run():
        summary = {
            "request_id": REQUEST, "kind": "environment_change", "environment_id": ENVIRONMENT,
            "actor": "operator", "reason": "test review", "timestamp_unix_ms": 1,
            "expires_at_unix_ms": 1000, "decision": "pending", "application": "pending",
        }
        response: dict[str, object] = {"requests": [summary], "next_cursor": 9}
        async with gateway(response=response) as (url, received), Hypervisor(url, "gateway-token") as hv:
            page = await hv.requests.list(after=7, limit=1, pending_only=True)
            assert page.next_cursor == 9
            assert page.requests[0].request_id == REQUEST
            response.clear()
            response.update({
                "summary": summary, "observed_revision": "0123456789ab",
                "proposed": {
                    "availability": "complete", "purged_variables": [],
                    "specification": {"name": "development", "variables": {"EDITOR": "emacs"}},
                },
                "targets": [],
            })
            detail = await hv.requests.info(REQUEST)
            assert detail.observed_revision == "0123456789ab"
            assert detail.proposed.specification is not None
            assert detail.proposed.specification.variables == {"EDITOR": "emacs"}
            with pytest.raises(ValueError, match="identity changed"):
                await hv.requests.info(ENVIRONMENT)
            assert [path for _, path, _ in received] == [
                "/requests/list?after=7&limit=1&pending_only=true",
                f"/requests/{REQUEST}/info", f"/requests/{ENVIRONMENT}/info",
            ]
    asyncio.run(run())


@pytest.mark.parametrize("is_change", [False, True])
def test_environment_mutations_preserve_explicit_deadline_and_cancellation(is_change):
    async def run():
        async with gateway(response=PENDING, response_delay=0.15) as (url, received), Hypervisor(url, "gateway-token", timeout=0.03) as hv:
            async def invoke(timeout: float | None = None):
                if is_change:
                    changed = models.EnvironmentChangeRequest.model_validate({
                        "request_id": REQUEST, "base_revision_id": ENVIRONMENT,
                        "specification": {"name": "development"}, "reason": "update blueprint",
                    })
                    return await hv.environments.change(ENVIRONMENT, changed, request_timeout=timeout)
                return await hv.environments.create(request(), request_timeout=timeout)
            assert (await invoke()).request_id == REQUEST
            with pytest.raises(TimeoutError):
                await invoke(0.03)
            with pytest.raises(TimeoutError):
                async with asyncio.timeout(0.03):
                    await invoke()
            path = f"/environments/{ENVIRONMENT}/changes" if is_change else "/environments/create"
            assert [received_path for _, received_path, _ in received] == [path] * 3
    asyncio.run(run())


@pytest.mark.parametrize("response,valid", [
    (PENDING, True), ({**PENDING, "request_id": ENVIRONMENT}, False),
    ({**PENDING, "environment_id": REQUEST}, False),
])
def test_changes_preserve_base_and_refuse_substituted_identities(response, valid):
    async def run():
        base = "00000000-0000-4000-8000-000000000003"
        change = models.EnvironmentChangeRequest.model_validate({
            "request_id": REQUEST, "base_revision_id": base,
            "specification": {"name": "development"}, "reason": "update blueprint",
        })
        async with gateway(response=response) as (url, received), Hypervisor(url, "gateway-token") as hv:
            if valid:
                assert (await hv.environments.change(ENVIRONMENT, change)).application == models.RequestApplicationState.PENDING
            else:
                with pytest.raises(ValueError, match="identity changed"):
                    await hv.environments.change(ENVIRONMENT, change)
            assert len(received) == 1
            method, path, body = received[0]
            assert (method, path) == ("POST", f"/environments/{ENVIRONMENT}/changes")
            assert body["request_id"] == REQUEST
            assert body["base_revision_id"] == base
    asyncio.run(run())
