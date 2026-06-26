"""Shared route-matrix assertions for profile-owned UI/API surfaces."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Callable


@dataclass(frozen=True)
class RouteSpec:
    method: str
    path: str
    body: dict[str, Any] | None
    required_keys: frozenset[str]
    response_kind: type


def enforcement_payload(action: str = "allow") -> dict[str, Any]:
    return {
        "rules_toml": f"""
[profiles.rules.route_matrix_{action}]
name = "route_matrix_{action}"
action = "{action}"
detection_level = "informational"
match = 'http.host == "route-matrix.example"'
""".strip(),
        "event": {
            "event_type": "http.request",
            "http_host": "route-matrix.example",
        },
    }


def profile_route_specs(profile_id: str) -> list[RouteSpec]:
    return [
        RouteSpec("GET", f"/profiles/{profile_id}/info", None, frozenset({"profile", "obom"}), dict),
        RouteSpec(
            "GET",
            f"/profiles/{profile_id}/assets/status",
            None,
            frozenset({"profile_id", "ready", "assets", "missing_assets", "invalid_assets", "manifest"}),
            dict,
        ),
        RouteSpec(
            "GET",
            f"/profiles/{profile_id}/assets/info",
            None,
            frozenset({"profile_id", "current_arch", "refresh_policy", "current_assets"}),
            dict,
        ),
        RouteSpec(
            "GET",
            f"/profiles/{profile_id}/enforcement/info",
            None,
            frozenset({"profile_id", "rule_count", "action_counts"}),
            dict,
        ),
        RouteSpec(
            "GET",
            f"/profiles/{profile_id}/enforcement/rules/list",
            None,
            frozenset({"profile_id", "rules"}),
            dict,
        ),
        RouteSpec(
            "POST",
            f"/profiles/{profile_id}/enforcement/evaluate",
            enforcement_payload("allow"),
            frozenset({"event"}),
            dict,
        ),
        RouteSpec(
            "GET",
            f"/profiles/{profile_id}/detection/info",
            None,
            frozenset({"profile_id", "rule_count", "detection_rule_count"}),
            dict,
        ),
        RouteSpec(
            "GET",
            f"/profiles/{profile_id}/detection/rules/list",
            None,
            frozenset({"profile_id", "rules"}),
            dict,
        ),
        RouteSpec(
            "POST",
            f"/profiles/{profile_id}/detection/evaluate",
            enforcement_payload("allow"),
            frozenset({"event"}),
            dict,
        ),
        RouteSpec(
            "GET",
            f"/profiles/{profile_id}/plugins/info",
            None,
            frozenset({"scope", "plugin_count", "enabled_count"}),
            dict,
        ),
        RouteSpec(
            "GET",
            f"/profiles/{profile_id}/plugins/list",
            None,
            frozenset({"scope", "plugins"}),
            dict,
        ),
        RouteSpec(
            "GET",
            f"/profiles/{profile_id}/plugins/credential_broker/info",
            None,
            frozenset({"id", "name", "scope", "description", "stage", "version", "runtime"}),
            dict,
        ),
        RouteSpec(
            "GET",
            f"/profiles/{profile_id}/plugins/credential_broker/credentials/info",
            None,
            frozenset({"scope", "plugin_id", "store", "inventory", "grants", "corp_constraints"}),
            dict,
        ),
        RouteSpec(
            "POST",
            f"/profiles/{profile_id}/plugins/credential_broker/credentials/reload",
            {},
            frozenset({"scope", "plugin_id", "store", "inventory", "grants", "corp_constraints"}),
            dict,
        ),
        RouteSpec(
            "GET",
            f"/profiles/{profile_id}/mcp/info",
            None,
            frozenset({"profile_id", "server_count", "builtin_local_enabled"}),
            dict,
        ),
        RouteSpec(
            "GET",
            f"/profiles/{profile_id}/mcp/default/info",
            None,
            frozenset({"action", "source", "rule_id"}),
            dict,
        ),
        RouteSpec("GET", f"/profiles/{profile_id}/mcp/servers/list", None, frozenset(), list),
        RouteSpec(
            "GET",
            f"/profiles/{profile_id}/mcp/servers/local/tools/list",
            None,
            frozenset(),
            list,
        ),
    ]


def assert_payload_contract(spec: RouteSpec, payload: Any) -> None:
    assert isinstance(payload, spec.response_kind), (spec.path, payload)
    if isinstance(payload, dict):
        assert "error" not in payload, (spec.path, payload)
        assert spec.required_keys <= set(payload), (spec.path, payload)
    else:
        assert not spec.required_keys, spec


def assert_profile_route_matrix(
    *,
    profiles: tuple[str, ...],
    request: Callable[[RouteSpec], Any],
) -> None:
    for profile_id in profiles:
        for spec in profile_route_specs(profile_id):
            assert_payload_contract(spec, request(spec))
