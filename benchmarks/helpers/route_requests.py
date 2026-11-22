"""Seed authenticated benchmark fixtures and require the expected route responses."""

from __future__ import annotations

import json
import secrets
import uuid
from collections.abc import Callable
from dataclasses import dataclass
from typing import TYPE_CHECKING

from helpers.stopped_workspace import BODY_ROUTE, BODY_URL, VM_ID, assert_event_bodies
from helpers.gateway import TcpHttpClient

if TYPE_CHECKING:
    import psutil
    from helpers.http_transport import Transport

MANAGED_REQUEST_ID = "fc120782-2272-4b3d-a2e6-a26ee2f8ab61"
MANAGED_TOKEN = "af" * 32
AGENT_ISSUE_ID = "d61cc17c-d5c7-410f-b2f8-767a2f8d5405"
AGENT_REVOKE_ID = "2bba540e-c785-43f7-9c9b-e19225426132"
LIVE_ROUTES = {"/vms/{id}/exposures", "/vms/{id}/security/status"}
VM_READINESS = "/vms/{id}/readiness"
VM_READINESS_URL = f"/vms/{VM_ID}/readiness?intent=start"
ENV_CREATE = "/environments/create"
ENV_CHANGE = "/environments/{id}/changes"
ENV_PROMOTE = "/environments/{id}/promote"
ENV_INFO = "/environments/{id}/info"
ENV_READINESS = "/environments/{id}/revisions/{revision_id}/readiness"
ENV_ROUTES = {ENV_CREATE, ENV_CHANGE, ENV_PROMOTE, ENV_INFO, ENV_READINESS}


@dataclass(frozen=True)
class EnvironmentFixture:
    original: dict[str, str]
    current_revision_id: str
    replays: dict[str, tuple[bytes, dict]]


def seed_environment(client: TcpHttpClient, gateway_url: str, *, measure_promotion: bool) -> EnvironmentFixture:
    """Use the operator gateway to commit an environment and then advance it."""
    create = {
        "request_id": str(uuid.uuid4()),
        "specification": {"name": "route-benchmark"},
        "reason": "create the route benchmark environment",
    }
    created = client.post(ENV_CREATE, create)
    if created["application"] != "applied" or created["request_id"] != create["request_id"]:
        raise RuntimeError("benchmark environment creation did not apply")
    original = created["result"]
    change_path = ENV_CHANGE.format(id=original["environment_id"])
    change = {
        "request_id": str(uuid.uuid4()),
        "base_revision_id": original["revision_id"],
        "specification": {"name": "route-benchmark", "description": "second revision"},
        "reason": "advance the route benchmark environment",
    }
    changed = client.post(change_path, change)
    if changed["application"] != "applied" or changed["request_id"] != change["request_id"]:
        raise RuntimeError("benchmark environment change did not apply")
    revised = changed["result"]
    if revised["environment_id"] != original["environment_id"] or revised["revision_id"] == original["revision_id"]:
        raise RuntimeError("benchmark environment did not advance to a new revision")
    replays = {
        ENV_CREATE: (json.dumps(create).encode(), created),
        change_path: (json.dumps(change).encode(), changed),
    }
    if measure_promotion:
        path = ENV_PROMOTE.format(id=revised["environment_id"])
        body, receipt = seed_promotion(client, gateway_url, revised, change_path)
        replays[path] = (json.dumps(body).encode(), receipt)
        revised = receipt["result"]
    return EnvironmentFixture(original, revised["revision_id"], replays)


def seed_promotion(
    client: TcpHttpClient, gateway_url: str, parent: dict[str, str], change_path: str,
) -> tuple[dict, dict]:
    """Use a live sandbox to obtain an approved fork, then promote it before timing."""
    session_id = client.post("/vms/create", {"name": "promotion-benchmark", "environment": parent})["id"]
    try:
        generation = client.get(f"/vms/{session_id}/info")["environment"]["owner_generation"]
        secret = secrets.token_hex(32)
        access = client.post("/agent-access", {
            "request_id": str(uuid.uuid4()), "ownership_token": secret, "lease_seconds": 300,
            "scope": {"operations": ["propose"], "sessions": [
                {"session_id": session_id, "owner_generation": generation},
            ]},
        })["access"]["id"]
        proposal_id = str(uuid.uuid4())
        agent = TcpHttpClient(gateway_url, f"{access}.{secret}")
        try:
            proposed = agent.post(change_path, {
                "request_id": proposal_id, "base_revision_id": parent["revision_id"],
                "specification": {"name": "route-benchmark", "description": "reviewed benchmark fork"},
                "reason": "prepare a fork to benchmark receipt replay",
            })
        finally:
            agent.close()
        approved = client.post(f"/requests/{proposal_id}/resolve", {
            "observed_revision": proposed["observed_revision"], "decision": "approve",
            "reason": "approve the route benchmark fork",
        })
        body = {
            "request_id": str(uuid.uuid4()), "base_revision_id": parent["revision_id"],
            "source": approved["result"], "reason": "promote the route benchmark fork",
        }
        receipt = client.post(ENV_PROMOTE.format(id=parent["environment_id"]), body)
        if (receipt["application"] != "applied" or receipt["request_id"] != body["request_id"]
                or receipt["result"]["environment_id"] != parent["environment_id"]
                or receipt["result"]["revision_id"] == parent["revision_id"]):
            raise RuntimeError("benchmark fork promotion did not advance the parent")
        return body, receipt
    finally:
        client.delete(f"/vms/{session_id}/delete")


def request_path(
    route: str, live_session_id: str | None = None, *, environment: EnvironmentFixture | None = None
) -> str:
    """Resolve a configured route template against its exact fixture."""
    if route == VM_READINESS:
        return VM_READINESS_URL
    if route in ENV_ROUTES:
        if environment is None:
            raise ValueError(f"measured route requires an environment fixture: {route}")
        return route.format(
            id=environment.original["environment_id"], revision_id=environment.original["revision_id"]
        )
    if route == "/agent-access/{id}":
        return f"/agent-access/{AGENT_REVOKE_ID}"
    if route == BODY_ROUTE:
        return BODY_URL
    if route in LIVE_ROUTES and live_session_id is None:
        raise ValueError(f"measured route requires a live owner fixture: {route}")
    subject = live_session_id if route in LIVE_ROUTES else VM_ID
    path = route.replace("{id}", subject).replace("{request_id}", MANAGED_REQUEST_ID)
    if "{" in path or "}" in path:
        raise ValueError(f"measured route has no successful fixture: {route}")
    return path


def request_input(path: str) -> tuple[str, bytes | None]:
    """Return the method and body for one repeatable benchmark request."""
    if path == "/agent-access":
        return "POST", json.dumps({
            "request_id": AGENT_ISSUE_ID, "ownership_token": MANAGED_TOKEN,
            "scope": {"operations": ["discover"]}, "lease_seconds": 86400,
        }).encode()
    if path == f"/agent-access/{AGENT_REVOKE_ID}":
        return "DELETE", None
    if path.endswith("/claim"):
        return "POST", json.dumps({"ownership_token": MANAGED_TOKEN}).encode()
    if path.endswith("/close"):
        payload = {"ownership_token": MANAGED_TOKEN, "outcome": "cancelled"}
        return "POST", json.dumps(payload).encode()
    return "GET", None


@dataclass(frozen=True)
class Target:
    label: str
    transport_factory: Callable[[], Transport]
    headers: dict[str, str]
    cpu_processes: dict[str, psutil.Process]
    environment: EnvironmentFixture | None = None


def request(transport: Transport, target: Target, path: str) -> None:
    """Require one successful JSON response."""
    method, request_body = request_input(path)
    environment = target.environment
    replay = environment.replays.get(path) if environment is not None else None
    if replay is not None:
        method, request_body = "POST", replay[0]
    headers = dict(target.headers)
    if request_body is not None:
        headers["Content-Type"] = "application/json"
    status, _, body = transport.request(
        method, path, headers=headers, body=request_body
    )
    if not 200 <= status < 300:
        raise RuntimeError(
            f"{target.label} {path} returned HTTP {status}: {body[:500]!r}"
        )
    try:
        payload = json.loads(body)
    except json.JSONDecodeError as error:
        raise RuntimeError(
            f"{target.label} {path} returned non-JSON: {body[:500]!r}"
        ) from error
    if path == BODY_URL:
        assert_event_bodies(payload)
    if path == VM_READINESS_URL and payload["selection"]["id"] != VM_ID:
        raise RuntimeError("benchmark VM readiness selected a different VM")
    if replay is not None and payload != replay[1]:
        raise RuntimeError(f"{path} did not return the original applied receipt")
    if environment is not None:
        if path == request_path(ENV_INFO, environment=environment):
            identity = payload["document"]["identity"]
            if identity["id"] != environment.original["environment_id"] or identity["revision_id"] != environment.current_revision_id:
                raise RuntimeError("benchmark environment info did not return the committed head")
        if path == request_path(ENV_READINESS, environment=environment) and payload["environment"] != environment.original:
            raise RuntimeError("benchmark environment readiness did not return the requested historical revision")
