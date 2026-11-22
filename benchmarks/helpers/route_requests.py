"""Seed authenticated benchmark fixtures and require the expected route responses."""

from __future__ import annotations

import json
import uuid
from collections.abc import Callable
from dataclasses import dataclass
from typing import TYPE_CHECKING

from helpers.stopped_workspace import BODY_ROUTE, BODY_URL, VM_ID, assert_event_bodies

if TYPE_CHECKING:
    import psutil
    from helpers.gateway import TcpHttpClient
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
ENV_INFO = "/environments/{id}/info"
ENV_READINESS = "/environments/{id}/revisions/{revision_id}/readiness"
ENV_ROUTES = {ENV_CREATE, ENV_CHANGE, ENV_INFO, ENV_READINESS}


@dataclass(frozen=True)
class EnvironmentFixture:
    original: dict[str, str]
    current_revision_id: str
    replays: dict[str, tuple[bytes, dict]]


def seed_environment(client: TcpHttpClient) -> EnvironmentFixture:
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
    return EnvironmentFixture(original, revised["revision_id"], {
        ENV_CREATE: (json.dumps(create).encode(), created),
        change_path: (json.dumps(change).encode(), changed),
    })


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
