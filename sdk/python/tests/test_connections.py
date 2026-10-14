"""Friendly connection resources preserve explicit requests and trusted identity."""
from __future__ import annotations

import asyncio
from typing import Any
from urllib.parse import quote

import pytest
from capsem import HttpError, Hypervisor, models

from .test_contract import SCHEMAS, sample
from .test_images import gateway

OWNER = "bb120782-2272-4b3d-a2e6-a26ee2f8ab61"
REQUEST = "fc120782-2272-4b3d-a2e6-a26ee2f8ab61"
SESSION = "session /?&é"
CONNECTION = "connection /?&é"
TRANSACTION = "transaction /?&é"
GRANT = "grant /?&é"


def test_connection_pages_forward_limit_and_cursor_to_authenticated_gateway():
    async def run():
        response = {"connections": [], "scope_profiles": [], "next_cursor": None}
        async with gateway(response=response) as (url, received), Hypervisor(url, "gateway-token") as hv:
            await hv.connections.list(limit=2, cursor=REQUEST)
            assert received[0][1] == f"/connections?limit=2&cursor={REQUEST}"
    asyncio.run(run())


def record(name: str) -> dict[str, Any]:
    value = sample(SCHEMAS[name])
    assert isinstance(value, dict)
    return value


def connection():
    return {**record("ConnectionSummary"), "id": CONNECTION}


def transaction():
    return {**record("AuthorizationTransaction"), "id": TRANSACTION}


def grant():
    return {**record("SessionGrantSummary"), "id": GRANT, "session_id": SESSION,
            "owner_generation": OWNER, "connection_id": CONNECTION, "capability_ids": ["agy"]}


def grant_request():
    return models.CreateSessionGrantRequest(
        request_id=REQUEST, expected_owner_generation=OWNER, connection_id=CONNECTION,
        capability_ids=["agy"], expires_at=None,
    )


@pytest.mark.parametrize("call,response", [
    (lambda hv: hv.connections.authorization(TRANSACTION), {**transaction(), "id": "other"}),
    (lambda hv: hv.connections.cancel_authorization(TRANSACTION), {**transaction(), "id": "other"}),
    (lambda hv: hv.connections.get(CONNECTION), {**connection(), "id": "other"}),
    (lambda hv: hv.connections.disconnect(CONNECTION), {"connection": {**connection(), "id": "other"}, "local_authority": "revoked", "revoked_grant_count": 1}),
    (lambda hv: hv.vm(id=SESSION).credential_grants.revoke(GRANT), {"grant": {**grant(), "id": "other"}, "local_authority": "revoked"}),
    (lambda hv: hv.vm(id=SESSION).credential_grants.revoke(GRANT), {"grant": {**grant(), "session_id": "other"}, "local_authority": "revoked"}),
    (lambda hv: hv.vm(id=SESSION).credential_grants.list(), {"session_id": "other", "owner_generation": OWNER, "grants": [], "denied_capability_ids": []}),
    (lambda hv: hv.vm(id=SESSION).credential_grants.list(), {"session_id": SESSION, "owner_generation": "other", "grants": [grant()], "denied_capability_ids": []}),
])
def test_reads_and_revoke_refuse_substituted_identity(call, response):
    async def run():
        async with gateway(response=response) as (url, received), Hypervisor(url, "gateway-token") as hv:
            with pytest.raises(ValueError, match="changed"):
                await call(hv)
            assert len(received) == 1
    asyncio.run(run())


def test_all_nine_friendly_calls_preserve_explicit_authenticated_requests():
    async def run():
        authorize = models.AuthorizeGoogleRequest(
            request_id=REQUEST, scope_profile_id="agy-native", reconnect_connection_id=CONNECTION,
        )
        cases = [
            ("GET", "/connections", {"connections": [connection()], "scope_profiles": [], "next_cursor": None},
             lambda hv: hv.connections.list(), None),
            ("POST", "/connections/google/authorize", {"transaction": transaction(), "authorization_url": "https://accounts.google.com/consent"},
             lambda hv: hv.connections.authorize_google(authorize), authorize.model_dump(mode="json")),
            ("GET", "/connections/authorizations/" + quote(TRANSACTION, safe=""), transaction(),
             lambda hv: hv.connections.authorization(TRANSACTION), None),
            ("DELETE", "/connections/authorizations/" + quote(TRANSACTION, safe=""), transaction(),
             lambda hv: hv.connections.cancel_authorization(TRANSACTION), None),
            ("GET", "/connections/" + quote(CONNECTION, safe=""), connection(),
             lambda hv: hv.connections.get(CONNECTION), None),
            ("DELETE", "/connections/" + quote(CONNECTION, safe=""), {"connection": connection(), "local_authority": "revoked", "revoked_grant_count": 1},
             lambda hv: hv.connections.disconnect(CONNECTION), None),
            ("GET", "/vms/" + quote(SESSION, safe="") + "/credential-grants", {"session_id": SESSION, "owner_generation": OWNER, "grants": [grant()], "denied_capability_ids": []},
             lambda hv: hv.vm(id=SESSION).credential_grants.list(), None),
            ("POST", "/vms/" + quote(SESSION, safe="") + "/credential-grants", grant(),
             lambda hv: hv.vm(id=SESSION).credential_grants.create(grant_request()), grant_request().model_dump(mode="json")),
            ("DELETE", "/vms/" + quote(SESSION, safe="") + "/credential-grants/" + quote(GRANT, safe=""), {"grant": grant(), "local_authority": "revoked"},
             lambda hv: hv.vm(id=SESSION).credential_grants.revoke(GRANT), None),
        ]
        for method, path, response, call, body in cases:
            async with gateway(response=response) as (url, received), Hypervisor(url, "gateway-token") as hv:
                await call(hv)
                assert received == [(method, path, body)]
    asyncio.run(run())


@pytest.mark.parametrize("field,value", [
    ("session_id", "other"), ("owner_generation", "other"), ("connection_id", "other"),
    ("capability_ids", ["other"]), ("capability_ids", ["agy", "agy"]),
])
def test_grant_creation_refuses_substituted_authority(field, value):
    async def run():
        async with gateway(response={**grant(), field: value}) as (url, received), Hypervisor(url, "gateway-token") as hv:
            with pytest.raises(ValueError, match=r"grant.*changed"):
                await hv.vm(id=SESSION).credential_grants.create(grant_request())
            assert len(received) == 1
    asyncio.run(run())


def test_refused_consent_is_not_replayed():
    async def run():
        async with gateway(response={"error": "credential request refused", "code": "configuration_required"}, status=503) as (url, received), Hypervisor(url, "gateway-token") as hv:
            request = models.AuthorizeGoogleRequest(request_id=REQUEST, scope_profile_id="agy-native", reconnect_connection_id=None)
            with pytest.raises(HttpError) as refusal:
                await hv.connections.authorize_google(request)
            assert refusal.value.status == 503
            assert refusal.value.credential_code is models.CredentialErrorCode.CONFIGURATION_REQUIRED
            assert len(received) == 1
    asyncio.run(run())


@pytest.mark.parametrize("body", ["not-json", "null", "[]", '{"code":"future_code"}', '{}'])
def test_unknown_credential_error_envelopes_preserve_http_error(body):
    error = HttpError(503, body)
    assert error.credential_code is None
    assert error.status == 503 and error.body == body


def test_caller_deadline_also_bounds_name_resolution():
    async def run():
        response = {"sandboxes": [{**record("SandboxInfo"), "name": "named", "id": SESSION}]}
        async with gateway(response=response, response_delay=0.1) as (url, received), Hypervisor(url, "gateway-token") as hv:
            with pytest.raises(TimeoutError):
                await hv.vm(name="named").credential_grants.list(request_timeout=0.02)
            assert [(method, path) for method, path, _ in received] == [("GET", "/vms/list")]
    asyncio.run(run())


def test_response_checks_use_the_original_request_after_caller_mutation():
    async def run():
        request = grant_request()
        async with gateway(response={**grant(), "capability_ids": ["other"]},
                           on_request=lambda: setattr(request, "capability_ids", ["other"])) as (url, received), Hypervisor(url, "gateway-token") as hv:
            with pytest.raises(ValueError, match=r"grant.*changed"):
                await hv.vm(id=SESSION).credential_grants.create(request)
            assert received[0][2]["capability_ids"] == ["agy"]
            assert request.capability_ids == ["other"]
    asyncio.run(run())
