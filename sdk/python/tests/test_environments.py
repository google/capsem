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
           "archived": False, "timestamp_unix_ms": 1, "actor": "operator", "reason": "approved", "request_id": REQUEST}


def request():
    return models.EnvironmentCreateRequest.model_validate({
        "request_id": REQUEST, "specification": {"name": "development"}, "reason": "prepare blueprint",
    })


def test_promotion_submits_the_reviewed_fork_and_matches_the_receipt():
    async def run():
        body = models.EnvironmentPromotionRequest(
            request_id=REQUEST, base_revision_id=ENVIRONMENT, reason="promote the reviewed fork",
            source=models.EnvironmentRevisionReference(
                environment_id="00000000-0000-4000-8000-000000000003",
                revision_id="00000000-0000-4000-8000-000000000004",
            ),
        )
        async with gateway(response=PENDING) as (url, received), Hypervisor(url, "gateway-token") as hv:
            assert (await hv.environments.promote(ENVIRONMENT, body)).request_id == REQUEST
            assert received[-1] == (
                "POST", f"/environments/{ENVIRONMENT}/promote", body.model_dump(mode="json"),
            )
            with pytest.raises(ValueError, match="promotion identity changed"):
                await hv.environments.promote(body.source.environment_id, body)
    asyncio.run(run())

def test_application_preview_preserves_the_reviewed_generation():
    async def run():
        reference = {"environment_id": ENVIRONMENT, "revision_id": REQUEST}
        target = {"session_id": ENVIRONMENT, "generation": REQUEST, "base": reference}
        request = {"base_revision_id": REQUEST, "revision_id": REQUEST, "targets": [target]}
        preview = {"target": target, "boot": reference, "live_policy": reference,
                   "live_changes": ["access_rules"], "next_start_changes": ["variables"]}
        response = {"targets": [preview]}
        async with gateway(response=response) as (url, received), Hypervisor(url, "gateway-token") as hv:
            body = models.EnvironmentApplyPreviewRequest.model_validate(request)
            assert (await hv.environments.preview_application(ENVIRONMENT, body)).targets[0].next_start_changes == [
                "variables",
            ]
            assert received[-1] == ("POST", f"/environments/{ENVIRONMENT}/apply/preview", request)
            preview["target"] = {**target, "generation": ENVIRONMENT}
            with pytest.raises(ValueError, match="review selection changed"):
                await hv.environments.preview_application(ENVIRONMENT, body)
    asyncio.run(run())


def test_run_uses_an_exact_saved_revision_and_rejects_inline_variables():
    async def run():
        output = {"stdout": {"encoding": "utf8", "data": "pinned"},
                  "stderr": {"encoding": "utf8", "data": ""}, "exit_code": 0}
        async with gateway(response=output) as (url, received), Hypervisor(url, "gateway-token") as hv:
            reference = models.EnvironmentRevisionReference(environment_id=ENVIRONMENT, revision_id=REQUEST)
            assert (await hv.run("printf test", environment=reference)).exit_code == 0
            with pytest.raises(ValueError, match="inline variables"):
                await hv.run("printf test", environment=reference, env={})
            await hv.run("printf test")
            assert [path for _, path, _ in received] == ["/run", "/run"]
            assert received[0][2]["environment"] == reference.model_dump(mode="json")
            assert "environment" not in received[1][2]
    asyncio.run(run())


def test_selected_skill_methods_correlate_revision_and_pin():
    async def run():
        reference = models.EnvironmentRevisionReference(environment_id=ENVIRONMENT, revision_id=REQUEST)
        revision = reference.model_dump(mode="json")
        selection = {"id": "review", "version": "1.0.0", "catalog_reference": "registry.example/skills:stable",
                     "resolved_digest": "sha256:" + "0" * 64}
        metadata = {"id": "review", "version": "1.0.0", "name": "Review", "publisher": "Fixture",
                    "provenance": "Fixture catalog", "license": "Apache-2.0",
                    "compatibility": {"min_runtime_contract": 1}, "status": "active"}
        files = {"SKILL.md": "review fixture\n", "references/notes.md": "notes\n"}
        response: dict[str, object] = {"revision": revision, "discovery_fresh": False, "skills": [
            {"selection": selection, "metadata": metadata, "availability": "available"},
        ]}
        async with gateway(response=response) as (url, received), Hypervisor(url, "gateway-token") as hv:
            page = await hv.environments.skills(reference, search="fixture", limit=12)
            assert page.skills[0].selection.id == "review" and page.discovery_fresh is False
            assert 'search=fixture' in received[-1][1] and 'limit=12' in received[-1][1]
            response.clear()
            response.update(revision=revision, selection=selection, metadata=metadata, files=files, discovery_fresh=False)
            assert (await hv.environments.skill(reference, "review")).files == files
            assert f"/environments/{ENVIRONMENT}/skills/review/info?revision_id={REQUEST}" == received[-1][1]
            metadata["version"] = "2.0.0"
            with pytest.raises(ValueError, match="skill identity changed"):
                await hv.environments.skill(reference, "review")
    asyncio.run(run())


def test_vm_skill_reads_bind_sandbox_id_and_applied_revision():
    async def run():
        revision = {"environment_id": ENVIRONMENT, "revision_id": REQUEST}
        selection = {"id": "review", "version": "1.0.0", "catalog_reference": "registry.example/skills:stable",
                     "resolved_digest": "sha256:" + "0" * 64}
        metadata = {"id": "review", "version": "1.0.0", "name": "Review", "publisher": "Fixture",
                    "provenance": "Fixture catalog", "license": "Apache-2.0",
                    "compatibility": {"min_runtime_contract": 1}, "status": "active"}
        scope = {"vm_id": "vm-1", "owner_generation": REQUEST}
        detail = {"revision": revision, "selection": selection, "metadata": metadata,
                  "files": {"SKILL.md": "review fixture\n"}, "discovery_fresh": True}
        response: dict[str, object] = {**scope, "revision": revision, "skills": [], "discovery_fresh": True}
        async with gateway(response=response) as (url, received), Hypervisor(url, "gateway-token") as hv:
            vm = hv.vm(id="vm-1")
            assert (await vm.skills(revision_id=REQUEST, limit=1)).owner_generation == REQUEST
            assert received[-1][1].startswith("/vms/vm-1/skills/list?")
            response["revision"] = None
            with pytest.raises(ValueError, match="revision identity changed"):
                await vm.skills(revision_id=REQUEST)
            assert (await vm.skills()).revision is None
            response.clear()
            response.update(**scope, skill=detail)
            assert (await vm.skill("review", revision_id=REQUEST)).skill.files == detail["files"]
            assert received[-1][1] == f"/vms/vm-1/skills/review/info?revision_id={REQUEST}"
            response["vm_id"] = "vm-2"
            with pytest.raises(ValueError, match="skill identity changed"):
                await vm.skill("review")
    asyncio.run(run())


def test_vm_mcp_inventory_preserves_identity_and_input_schema():
    async def run():
        scope = {"vm_id": "vm-1", "owner_generation": REQUEST}
        schema = {"type": "object", "properties": {"query": {"type": "string"}}}
        response: dict[str, object] = {**scope, "servers": []}
        async with gateway(response=response) as (url, received), Hypervisor(url, "gateway-token") as hv:
            vm = hv.vm(id="vm-1")
            assert (await vm.mcps()).owner_generation == REQUEST
            assert received[-1][1] == "/vms/vm-1/mcps"
            response["vm_id"] = "vm-2"
            with pytest.raises(ValueError, match="inventory identity changed"):
                await vm.mcps()
            response.clear()
            response.update(**scope, server_id="library", tools=[{
                "namespaced_name": "library__search", "original_name": "search",
                "input_schema": schema, "definition_digest": "observed",
            }])
            assert (await vm.mcp_tools("library")).tools[0].input_schema == schema
            assert received[-1][1] == "/vms/vm-1/mcps/library/tools"
            with pytest.raises(ValueError, match="inventory identity changed"):
                await vm.mcp_tools("other")
    asyncio.run(run())


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
            response.clear()
            response.update({**PENDING, "decision": "denied"})
            decision = models.RequestResolveRequest(
                observed_revision=detail.observed_revision,
                decision=models.RequestDecision.DENY,
                reason="keep the original configuration",
            )
            assert (await hv.requests.resolve(REQUEST, decision)).decision == models.RequestState.DENIED
            assert [path for _, path, _ in received] == [
                "/requests/list?after=7&limit=1&pending_only=true",
                f"/requests/{REQUEST}/info", f"/requests/{ENVIRONMENT}/info",
                f"/requests/{REQUEST}/resolve",
            ]
            assert received[-1][2] == decision.model_dump(mode="json")
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
