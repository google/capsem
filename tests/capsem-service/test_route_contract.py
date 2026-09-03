"""UDS route contract for profile-owned service API surfaces.

The route matrix is the service-side half of the UI/TUI contract. A route that
the clients depend on must be explicit at the service boundary before the
gateway is allowed to forward it.
"""

from __future__ import annotations

from typing import Any

from helpers.route_matrix import RouteSpec, assert_profile_route_matrix

PROFILES = ("code", "co-work")


def _uds_request(client: Any, spec: RouteSpec) -> Any:
    status, payload = client.call_json(spec.method, spec.path, spec.body, timeout=30)
    assert status == 200, (spec.path, status, payload)
    return payload


def test_profile_route_contract_exists_for_every_ui_profile(client: Any) -> None:
    listed = client.get("/profiles/list")
    listed_ids = {profile["id"] for profile in listed["profiles"]}
    assert set(PROFILES) <= listed_ids

    assert_profile_route_matrix(
        profiles=PROFILES,
        request=lambda spec: _uds_request(client, spec),
    )
