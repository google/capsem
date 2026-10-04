"""Route matrix for the settings-scope service API surfaces.

The UI and TUI build their asset, plugin and MCP pages from these routes. A
missing route, fallback route, 404, or 501 is a product bug.
"""

from __future__ import annotations

from typing import Any

from helpers.route_matrix import RouteSpec, assert_settings_route_matrix


def _uds_request(client: Any, spec: RouteSpec) -> Any:
    status, payload = client.call_json(spec.method, spec.path, spec.body, timeout=30)
    assert status == 200, (spec.path, status, payload)
    return payload


def test_settings_route_matrix_answers_every_ui_surface(client: Any) -> None:
    assert_settings_route_matrix(lambda spec: _uds_request(client, spec))
