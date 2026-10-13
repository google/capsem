"""Managed ownership semantics over real HTTP; native VM proof is separate."""

from __future__ import annotations

import asyncio
import re
from collections.abc import AsyncIterator
from contextlib import asynccontextmanager
from datetime import UTC, datetime, timedelta
from uuid import UUID

import pytest
from aiohttp import web
from capsem import HttpError, Hypervisor

GENERATION = "46a98f98-4683-4b09-b319-e235179d76bb"
VM_ID = "2b17c25b-7993-42f8-b16b-60c10dc8c60e"


class Service:
    def __init__(self) -> None:
        self.requests: list[tuple[str, dict, dict]] = []
        self.request_id: str | None = None
        self.token: str | None = None
        self.state = "pending"
        self.create_seen = asyncio.Event()
        self.close_seen = asyncio.Event()
        self.claim_seen = asyncio.Event()
        self.release_create = asyncio.Event()
        self.release_close = asyncio.Event()
        self.release_create.set()
        self.release_close.set()
        self.claim_generation: str | None = None
        self.fail_close = 0
        self.initial_missing = 0
        self.invalid_create_reply = False

    def ownership(self, generation: str = GENERATION) -> dict:
        return {"request_id": self.request_id, "generation": generation,
                "expires_at": (datetime.now(UTC) + timedelta(seconds=4)).isoformat(),
                "lease_seconds": 4, "renew_after_seconds": 1}

    def session(self) -> dict:
        return {"id": VM_ID, "name": VM_ID, "status": "Running", "persistent": False,
                "can_resume": False, "available_actions": ["stop", "delete"],
                "ownership": self.ownership()}

    async def handle(self, request: web.Request) -> web.Response:
        assert request.method == "POST"
        assert request.headers["Authorization"] == "Bearer gateway-bearer"
        body = await request.json()
        self.requests.append((request.raw_path, body, dict(request.headers)))
        if request.path == "/vms/create":
            assert self.request_id is None, "create must never replay"
            self.request_id = body["managed"]["request_id"]
            self.token = body["managed"]["ownership_token"]
            self.create_seen.set()
            await self.release_create.wait()
            if self.state == "closed":
                return web.json_response({"code": "duplicate_request", "error": "closed"}, status=409)
            self.state = "active"
            if self.invalid_create_reply:
                return web.Response(text="lost-create-response", content_type="application/json")
            return web.json_response(self.session())
        assert request.path == f"/vms/managed/{self.request_id}/" + request.path.rsplit("/", 1)[1]
        assert body["ownership_token"] == self.token
        if request.path.endswith("/claim"):
            if self.initial_missing:
                self.initial_missing -= 1
                return web.json_response({"code": "not_found", "error": "pending admission"}, status=404)
            self.claim_seen.set()
            return web.json_response({"ownership": self.ownership(self.claim_generation or GENERATION),
                                      "state": self.state, "session": self.session() if self.state == "active" else None})
        assert request.path.endswith("/close")
        self.close_seen.set()
        await self.release_close.wait()
        if self.fail_close:
            self.fail_close -= 1
            return web.json_response({"code": "cleanup_failed", "error": "cleanup pending"}, status=500)
        self.state = "closed"
        self.release_create.set()
        return web.json_response({"request_id": self.request_id, "generation": GENERATION,
                                  "state": "closed", "session_id": VM_ID, "evidence_retained": False})


@asynccontextmanager
async def gateway(service: Service) -> AsyncIterator[str]:
    app = web.Application()
    app.router.add_route("POST", "/{tail:.*}", service.handle)
    runner = web.AppRunner(app, shutdown_timeout=0.2)
    await runner.setup()
    try:
        await web.TCPSite(runner, "127.0.0.1", 0).start()
        yield f"http://127.0.0.1:{runner.addresses[0][1]}"
    finally:
        service.release_close.set()
        service.release_create.set()
        await runner.cleanup()


def test_managed_scope_renews_service_policy_and_awaits_only_its_owned_close() -> None:
    async def run() -> None:
        service = Service()
        async with gateway(service) as url, Hypervisor(url, "gateway-bearer") as hv:
            scope = hv.managed(image="docker://busybox", env={"MODE": "owned"})
            async with scope as vm:
                assert vm.id == VM_ID
                await asyncio.wait_for(service.claim_seen.wait(), 2)
                assert service.token is not None
                await hv.vm(name="unrelated-named-session").close()
                assert not service.close_seen.is_set(), "connection close must not acquire ownership"
                assert service.token not in repr(scope)
            assert service.state == "closed"
            assert service.token is not None and service.request_id is not None
            assert UUID(service.request_id).version == 4
            assert re.fullmatch("[0-9a-f]{64}", service.token)
            assert [path for path, _, _ in service.requests].count("/vms/create") == 1
            assert service.requests[-1][1]["outcome"] == "completed"
            assert all(service.token not in path and service.token not in str(headers)
                       for path, _, headers in service.requests)
            create = service.requests[0][1]
            assert create["name"] is None and create["persistent"] is False
            assert create["container"]["env"] == {"MODE": "owned"}
            assert "MODE" not in (create.get("env") or {})
            with pytest.raises(RuntimeError):
                async with scope:
                    pass
    asyncio.run(run())


@pytest.mark.parametrize("cancel_during_create", [False, True])
def test_cancelled_scope_joins_close_even_after_repeated_cancellation(cancel_during_create: bool) -> None:
    async def run() -> None:
        service = Service()
        service.release_close.clear()
        if cancel_during_create:
            service.release_create.clear()
        async with gateway(service) as url, Hypervisor(url, "gateway-bearer") as hv:
            entered = asyncio.Event()
            async def work() -> None:
                async with hv.managed():
                    entered.set()
                    await asyncio.Event().wait()
            task = asyncio.create_task(work())
            await asyncio.wait_for(service.create_seen.wait() if cancel_during_create else entered.wait(), 1)
            task.cancel()
            await asyncio.wait_for(service.close_seen.wait(), 1)
            task.cancel()
            await asyncio.sleep(0)
            assert not task.done(), "cancellation returned before the service cleanup barrier"
            service.release_close.set()
            with pytest.raises(asyncio.CancelledError):
                await asyncio.wait_for(task, 1)
            assert service.state == "closed"
            assert sum(path == "/vms/create" for path, _, _ in service.requests) == 1
            assert service.requests[-1][1]["outcome"] == "cancelled"
    asyncio.run(run())


def test_changed_generation_stops_the_scope_without_adopting_another_owner() -> None:
    async def run() -> None:
        service = Service()
        service.claim_generation = "67652132-695e-432a-ac42-0e0fb995ddfd"
        async with gateway(service) as url, Hypervisor(url, "gateway-bearer") as hv:
            with pytest.raises(RuntimeError, match="ownership"):
                async with hv.managed():
                    await asyncio.sleep(3)
            assert service.state == "closed"
            assert sum(path == "/vms/create" for path, _, _ in service.requests) == 1
            assert service.requests[-1][1]["outcome"] == "failed"
    asyncio.run(run())


def test_cleanup_failure_is_visible_and_explicit_retry_joins_the_original_close() -> None:
    async def run() -> None:
        service = Service()
        service.fail_close = 1
        async with gateway(service) as url, Hypervisor(url, "gateway-bearer") as hv:
            scope = hv.managed()
            with pytest.raises(HttpError) as failure:
                async with scope:
                    pass
            assert failure.value.status == 500
            assert service.state != "closed"
            await scope.close()
            assert service.state == "closed"
            assert sum(path == "/vms/create" for path, _, _ in service.requests) == 1
            assert sum(path.endswith("/close") for path, _, _ in service.requests) == 2
    asyncio.run(run())


def test_pending_admission_is_claimed_and_renewed_without_replaying_create() -> None:
    async def run() -> None:
        service = Service()
        service.release_create.clear()
        service.initial_missing = 1
        async with gateway(service) as url, Hypervisor(url, "gateway-bearer") as hv:
            scope = hv.managed()
            async def work() -> None:
                async with scope as vm:
                    assert vm.id == VM_ID
            task = asyncio.create_task(work())
            await asyncio.wait_for(service.claim_seen.wait(), 1)
            # Allow the claim response to finish: it may renew a Pending lease
            # before the create HTTP response supplies a VM handle.
            while scope.ownership is None:
                await asyncio.sleep(0)
            service.release_create.set()
            await asyncio.wait_for(task, 1)
            assert service.state == "closed"
            assert sum(path == "/vms/create" for path, _, _ in service.requests) == 1
            assert sum(path.endswith("/claim") for path, _, _ in service.requests) >= 2
    asyncio.run(run())


def test_uncertain_create_response_still_closes_the_original_owner() -> None:
    async def run() -> None:
        service = Service()
        service.invalid_create_reply = True
        async with gateway(service) as url, Hypervisor(url, "gateway-bearer") as hv:
            with pytest.raises(ValueError):
                async with hv.managed():
                    pytest.fail("an unverified create must not expose a VM handle")
            assert service.state == "closed"
            assert sum(path == "/vms/create" for path, _, _ in service.requests) == 1
            assert service.requests[-1][1]["outcome"] == "failed"
    asyncio.run(run())


def test_failed_renewal_closes_even_when_the_body_handles_cancellation() -> None:
    async def run() -> None:
        service = Service()
        service.claim_generation = "67652132-695e-432a-ac42-0e0fb995ddfd"
        body_observed_close = False
        async with gateway(service) as url, Hypervisor(url, "gateway-bearer") as hv:
            with pytest.raises(RuntimeError, match="ownership"):
                async with hv.managed():
                    try:
                        await asyncio.Event().wait()
                    except asyncio.CancelledError:
                        await asyncio.wait_for(service.close_seen.wait(), 1)
                        body_observed_close = True
            assert body_observed_close, "cleanup was deferred until the body timed out"
            assert service.state == "closed"
            assert sum(path.endswith("/close") for path, _, _ in service.requests) == 1
    asyncio.run(run())
