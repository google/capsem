"""Route matrix for profile-owned service API surfaces.

The UI and TUI must be able to build profile pages from explicit profile
routes. A missing route, fallback route, 404, or 501 is a product bug.
"""

from __future__ import annotations

from typing import Any

from helpers.route_matrix import RouteSpec, assert_profile_route_matrix

PROFILES = ("code", "co-work")


def _uds_request(client: Any, spec: RouteSpec) -> Any:
    status, payload = client.call_json(spec.method, spec.path, spec.body, timeout=30)
    assert status == 200, (spec.path, status, payload)
    return payload


def test_profile_route_matrix_exists_for_every_ui_profile(client: Any) -> None:
    listed = client.get("/profiles/list")
    listed_ids = {profile["id"] for profile in listed["profiles"]}
    assert set(PROFILES) <= listed_ids

    assert_profile_route_matrix(
        profiles=PROFILES,
        request=lambda spec: _uds_request(client, spec),
    )
