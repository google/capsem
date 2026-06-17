"""Security-rule route contract for first-party CEL facts."""

from __future__ import annotations

from typing import Any

from helpers.constants import CODE_PROFILE_ID
from helpers.service import ServiceInstance


def _evaluate(client: Any, rules_toml: str, event: dict[str, object]) -> dict[str, Any]:
    return client.post(
        f"/profiles/{CODE_PROFILE_ID}/enforcement/evaluate",
        {"rules_toml": rules_toml.strip(), "event": event},
        timeout=30,
    )


def test_evaluate_route_accepts_network_facts_and_local_ask_rule() -> None:
    service = ServiceInstance()
    try:
        service.start()
        client = service.client()
        evaluated = _evaluate(
            client,
            """
            [profiles.rules.local_network_ask]
            name = "local_network_ask"
            action = "ask"
            detection_level = "medium"
            match = 'http.host == "127.0.0.1" && ip.value == "127.0.0.1" && tcp.port == "3713"'
            """,
            {
                "event_type": "http.request",
                "http_host": "127.0.0.1",
                "http_path": "/v1/chat/completions",
                "ip_value": "127.0.0.1",
                "ip_version": "4",
                "tcp_port": "3713",
            },
        )

        event = evaluated["event"]
        assert event["event_type"] == "http.request"
        assert event["http"]["host"] == "127.0.0.1"
        assert event["http"]["path"] == "/v1/chat/completions"
        assert event["ip"] == {"value": "127.0.0.1", "version": "4"}
        assert event["tcp"] == {"port": "3713"}
        assert event["decision"]["effective"] == "ask"
        assert event["detections"][0]["rule_id"] == "profiles.rules.local_network_ask"
        assert event["detections"][0]["detection_level"] == "medium"
    finally:
        service.stop()


def test_evaluate_route_accepts_model_and_mcp_facts() -> None:
    service = ServiceInstance()
    try:
        service.start()
        client = service.client()

        model = _evaluate(
            client,
            """
            [profiles.rules.unknown_model_provider]
            name = "unknown_model_provider"
            action = "allow"
            detection_level = "informational"
            match = 'model.provider == "unknown" && model.request.valid == "true" && model.response.valid == "true"'
            """,
            {
                "event_type": "model.call",
                "model_provider": "unknown",
                "model_name": "gemma4:latest",
                "model_request_body": '{"messages":[{"role":"user","content":"hi"}]}',
                "model_response_body": '{"output_text":"hello"}',
            },
        )["event"]
        assert model["event_type"] == "model.call"
        assert model["model"]["provider"] == "unknown"
        assert model["model"]["name"] == "gemma4:latest"
        assert model["decision"]["effective"] == "allow"
        assert model["detections"][0]["rule_id"] == "profiles.rules.unknown_model_provider"

        mcp = _evaluate(
            client,
            """
            [profiles.rules.unknown_mcp_tool]
            name = "unknown_mcp_tool"
            action = "ask"
            detection_level = "low"
            match = 'mcp.server.name == "observed:127.0.0.1:3713/mcp" && mcp.tool_call.valid == "true" && mcp.tool_call.name.contains("fixture") && mcp.request.arguments.contains("email")'
            """,
            {
                "event_type": "mcp.tool_call",
                "mcp_method": "tools/call",
                "mcp_server_name": "observed:127.0.0.1:3713/mcp",
                "mcp_tool_call_name": "fixture_lookup",
                "mcp_request_preview": '{"params":{"arguments":{"query":"email report"}}}',
            },
        )["event"]
        assert mcp["event_type"] == "mcp.tool_call"
        assert mcp["mcp"]["server_name"] == "observed:127.0.0.1:3713/mcp"
        assert mcp["mcp"]["tool_call_name"] == "fixture_lookup"
        assert mcp["mcp"]["request"]["arguments"] == {"query": "email report"}
        assert mcp["decision"]["effective"] == "ask"
        assert mcp["detections"][0]["rule_id"] == "profiles.rules.unknown_mcp_tool"
    finally:
        service.stop()


def test_evaluate_route_rejects_unbacked_cel_roots() -> None:
    service = ServiceInstance()
    try:
        service.start()
        client = service.client()

        for root, condition in {
            "credential": 'credential.ref == "credential:blake3:test"',
            "snapshot": 'snapshot.action == "create"',
            "security": 'security.decision == "allow"',
        }.items():
            rejected = _evaluate(
                client,
                f"""
                [profiles.rules.bad_{root}]
                name = "bad_{root}"
                action = "allow"
                match = '{condition}'
                """,
                {"event_type": "http.request", "http_host": "example.com"},
            )
            assert "error" in rejected
            assert "not a first-party security-event root" in rejected["error"]
    finally:
        service.stop()
