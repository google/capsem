"""Shared route-matrix assertions for the settings-scope UI/API surfaces.

Assets, plugins and MCP are answered once for the whole service: policy is
the built-in defaults, the user's settings.toml and the corp config.
"""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from typing import Any

CREDENTIAL_BROKER_KEYS = frozenset({"plugin_id", "store", "inventory", "grants", "corp_constraints"})


@dataclass(frozen=True)
class RouteSpec:
    method: str
    path: str
    body: dict[str, Any] | None
    required_keys: frozenset[str]
    response_kind: type


def settings_route_specs() -> list[RouteSpec]:
    return [
        RouteSpec(
            "GET",
            "/assets/status",
            None,
            frozenset({"ready", "downloading", "current_arch", "assets", "errors", "manifest"}),
            dict,
        ),
        RouteSpec("GET", "/plugins/list", None, frozenset({"plugins"}), dict),
        RouteSpec(
            "GET",
            "/plugins/credential_broker/credentials/info",
            None,
            CREDENTIAL_BROKER_KEYS,
            dict,
        ),
        RouteSpec(
            "POST",
            "/plugins/credential_broker/credentials/reload",
            {},
            CREDENTIAL_BROKER_KEYS,
            dict,
        ),
        RouteSpec(
            "GET",
            "/mcp/info",
            None,
            frozenset({"server_count", "manual_server_count", "builtin_local_enabled"}),
            dict,
        ),
        RouteSpec("GET", "/mcp/default/info", None, frozenset({"action", "source", "rule_id"}), dict),
        RouteSpec("GET", "/mcp/servers/list", None, frozenset(), list),
        RouteSpec("GET", "/mcp/servers/local/tools/list", None, frozenset(), list),
    ]


def assert_payload_contract(spec: RouteSpec, payload: Any) -> None:
    assert isinstance(payload, spec.response_kind), (spec.path, payload)
    if isinstance(payload, dict):
        assert "error" not in payload, (spec.path, payload)
        assert spec.required_keys <= set(payload), (spec.path, payload)
    else:
        assert not spec.required_keys, spec


def assert_settings_route_matrix(request: Callable[[RouteSpec], Any]) -> None:
    for spec in settings_route_specs():
        assert_payload_contract(spec, request(spec))
