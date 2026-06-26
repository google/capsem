"""Ironbank proof for local-network and model-provider CEL facts.

These checks exercise the public profile enforcement route. They intentionally
do not inspect Rust internals: the route receives a security event shape and
returns the serialized event/decision ledger that UI, TUI, and automation use.
"""

from __future__ import annotations

from typing import Any

import pytest

from helpers.constants import CODE_PROFILE_ID
from helpers.service import ServiceInstance


pytestmark = pytest.mark.integration


def _evaluate(client: Any, rules_toml: str, event: dict[str, object]) -> dict[str, Any]:
    payload = client.post(
        f"/profiles/{CODE_PROFILE_ID}/enforcement/evaluate",
        {"rules_toml": rules_toml.strip(), "event": event},
        timeout=30,
    )
    assert set(payload) == {"event"}, payload
    return payload["event"]


def test_local_network_ip_tcp_facts_ask_by_default_blackbox() -> None:
    service = ServiceInstance()
    try:
        service.start()
        client = service.client()

        event = _evaluate(
            client,
            """
            [profiles.rules.local_network_ask]
            name = "local_network_ask"
            action = "ask"
            detection_level = "medium"
            match = 'ip.value == "10.0.0.7" && tcp.port == "8080"'
            """,
            {
                "event_type": "http.request",
                "http_host": "10.0.0.7",
                "http_path": "/admin",
                "ip_value": "10.0.0.7",
                "ip_version": "4",
                "tcp_port": "8080",
            },
        )

        assert event["event_type"] == "http.request"
        assert event["http"] == {
            "host": "10.0.0.7",
            "method": None,
            "path": "/admin",
            "query": None,
            "status": None,
            "body": None,
        }
        assert event["ip"] == {"value": "10.0.0.7", "version": "4"}
        assert event["tcp"] == {"port": "8080"}
        assert event["decision"] == {"effective": "ask"}
        assert event["detections"] == [
            {
                "source": "rule",
                "detection_level": "medium",
                "rule_id": "profiles.rules.local_network_ask",
                "plugin_id": None,
                "action": "ask",
                "plugin_mode": None,
                "reason": None,
            }
        ]
    finally:
        service.stop()


def test_ollama_local_backend_can_be_allowed_by_profile_rule_blackbox() -> None:
    service = ServiceInstance()
    try:
        service.start()
        client = service.client()

        event = _evaluate(
            client,
            """
            [profiles.rules.ollama_local_backend]
            name = "ollama_local_backend"
            action = "allow"
            detection_level = "informational"
            match = 'http.host == "local.ollama" && tcp.port == "11434"'
            """,
            {
                "event_type": "http.request",
                "http_host": "local.ollama",
                "http_path": "/api/chat",
                "ip_value": "127.0.0.1",
                "ip_version": "4",
                "tcp_port": "11434",
            },
        )

        assert event["event_type"] == "http.request"
        assert event["http"] == {
            "host": "local.ollama",
            "method": None,
            "path": "/api/chat",
            "query": None,
            "status": None,
            "body": None,
        }
        assert event["ip"] == {"value": "127.0.0.1", "version": "4"}
        assert event["tcp"] == {"port": "11434"}
        assert event["decision"] == {"effective": "allow"}
        assert event["detections"][0]["rule_id"] == "profiles.rules.ollama_local_backend"
        assert event["detections"][0]["detection_level"] == "informational"
        assert event["detections"][0]["action"] == "allow"
    finally:
        service.stop()


def test_unknown_provider_detection_uses_model_facts_blackbox() -> None:
    service = ServiceInstance()
    try:
        service.start()
        client = service.client()

        event = _evaluate(
            client,
            """
            [profiles.rules.unknown_provider_detect]
            name = "unknown_provider_detect"
            action = "allow"
            detection_level = "informational"
            match = 'model.provider == "unknown" && model.request.valid == "true" && model.response.valid == "true"'
            """,
            {
                "event_type": "model.call",
                "model_provider": "unknown",
                "model_name": "gemma4:latest",
                "model_request_body": '{"messages":[{"role":"user","content":"hello"}]}',
                "model_response_body": '{"message":{"content":"world"}}',
            },
        )

        assert event["event_type"] == "model.call"
        assert event["model"]["provider"] == "unknown"
        assert event["model"]["name"] == "gemma4:latest"
        assert event["model"]["request"] == {"valid": True}
        assert event["model"]["response"] == {"valid": True}
        assert event["model"]["tool_call"] == {"valid": False}
        assert event["decision"] == {"effective": "allow"}
        assert event["detections"] == [
            {
                "source": "rule",
                "detection_level": "informational",
                "rule_id": "profiles.rules.unknown_provider_detect",
                "plugin_id": None,
                "action": "allow",
                "plugin_mode": None,
                "reason": None,
            }
        ]
    finally:
        service.stop()
