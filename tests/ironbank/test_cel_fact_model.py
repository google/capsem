"""Black-box contract for the CEL fact model exposed by profile routes."""

from __future__ import annotations

from typing import Any

from helpers.constants import CODE_PROFILE_ID
from helpers.service import ServiceInstance


FORBIDDEN_FACTS = (
    "credential.",
    "snapshot.",
    "security.",
    "is_private(",
    "is_loopback(",
)


def _rules_by_id(client: Any) -> dict[str, dict[str, Any]]:
    response = client.get(f"/profiles/{CODE_PROFILE_ID}/enforcement/rules/list")
    assert response["profile_id"] == CODE_PROFILE_ID
    return {rule["rule_id"]: rule for rule in response["rules"]}


def test_profile_default_rules_are_visible_first_party_cel_rules() -> None:
    service = ServiceInstance()
    try:
        service.start()
        client = service.client()
        rules = _rules_by_id(client)

        for rule_id, action in {
            "profiles.rules.default_000_local_network": "ask",
            "profiles.rules.default_http": "allow",
            "profiles.rules.default_dns": "allow",
            "profiles.rules.default_mcp": "allow",
            "profiles.rules.default_model": "allow",
            "profiles.rules.default_file": "allow",
            "profiles.rules.default_process": "allow",
            "profiles.rules.default_unknown_model_provider": "allow",
            "profiles.rules.default_unknown_mcp_server": "allow",
        }.items():
            assert rule_id in rules
            assert rules[rule_id]["action"] == action
            assert rules[rule_id]["default_rule"] is True
            assert rules[rule_id]["priority"] > 1000
            assert rules[rule_id]["reason"]

        assert rules["profiles.rules.default_unknown_model_provider"]["detection_level"] == "informational"
        assert rules["profiles.rules.default_unknown_mcp_server"]["detection_level"] == "informational"
        local_condition = rules["profiles.rules.default_000_local_network"]["match"]
        assert "ip.value" in local_condition
        assert "http.host" in local_condition
        assert "mcp.server.name" in rules["profiles.rules.default_unknown_mcp_server"]["match"]

        for rule in rules.values():
            condition = rule["match"]
            assert not any(forbidden in condition for forbidden in FORBIDDEN_FACTS), (
                rule["rule_id"],
                condition,
            )
    finally:
        service.stop()


def test_evaluate_route_exercises_first_party_roots_without_fanout() -> None:
    service = ServiceInstance()
    try:
        service.start()
        client = service.client()
        response = client.post(
            f"/profiles/{CODE_PROFILE_ID}/enforcement/evaluate",
            {
                "rules_toml": """
                [profiles.rules.cross_root_model_probe]
                name = "cross_root_model_probe"
                action = "allow"
                detection_level = "informational"
                match = '''
                (http.host == "127.0.0.1" && tcp.port == "3713")
                || (model.provider == "unknown" && model.request.valid == "true")
                || (mcp.server.name == "observed:127.0.0.1:3713/mcp" && mcp.tool_call.valid == "true")
                '''
                """,
                "event": {
                    "event_type": "model.call",
                    "http_host": "127.0.0.1",
                    "tcp_port": "3713",
                    "model_provider": "unknown",
                    "model_request_body": '{"input":"hello"}',
                    "mcp_server_name": "observed:127.0.0.1:3713/mcp",
                    "mcp_tool_call_name": "fixture_lookup",
                },
            },
            timeout=30,
        )

        event = response["event"]
        assert event["event_type"] == "model.call"
        assert event["http"]["host"] == "127.0.0.1"
        assert event["tcp"]["port"] == "3713"
        assert event["model"]["provider"] == "unknown"
        assert event["mcp"]["tool_call_name"] == "fixture_lookup"
        assert event["decision"]["effective"] == "allow"
        assert [d["rule_id"] for d in event["detections"]] == [
            "profiles.rules.cross_root_model_probe"
        ]
    finally:
        service.stop()
