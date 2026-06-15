"""Ironbank plugin route and evaluation ledger contract tests."""

from __future__ import annotations

import pytest

from helpers.constants import CODE_PROFILE_ID
from helpers.service import ServiceInstance


pytestmark = pytest.mark.integration


RULES_TOML = """
[profiles.rules.eicar]
name = "eicar_rewrite_scan"
action = "allow"
detection_level = "high"
match = 'file.import.content.contains("EICAR")'
""".strip()


def _evaluate(client, import_content: str) -> dict:
    payload = {
        "rules_toml": RULES_TOML,
        "event": {
            "event_type": "file.import",
            "file_import_content": import_content,
        },
    }
    response = client.post(
        f"/profiles/{CODE_PROFILE_ID}/enforcement/evaluate",
        payload,
        timeout=30,
    )
    assert set(response) == {"event"}
    event = response["event"]
    assert event["event_type"] == "file.import"
    assert event["file"]["import_content"] is not None
    assert event["http"] is None
    assert event["dns"] is None
    assert event["mcp"] is None
    assert event["model"] is None
    assert event["process"] is None
    assert event["ip"] is None
    assert event["tcp"] is None
    assert event["udp"] is None
    return event


def _plugins_by_id(client) -> dict[str, dict]:
    body = client.get(f"/profiles/{CODE_PROFILE_ID}/plugins/list", timeout=30)
    assert body["scope"] == {"kind": "profile", "profile_id": CODE_PROFILE_ID}
    plugins = {plugin["id"]: plugin for plugin in body["plugins"]}
    assert {"credential_broker", "log_sanitizer", "dummy_pre_eicar", "dummy_post_allow"} <= set(
        plugins
    )
    return plugins


def _detection_sources(event: dict) -> set[tuple[str, str | None, str | None]]:
    return {
        (
            detection["source"],
            detection.get("rule_id"),
            detection.get("plugin_id"),
        )
        for detection in event["detections"]
    }


def test_plugin_routes_control_pre_post_logging_stages_and_evaluation_blackbox() -> None:
    service = ServiceInstance()
    client = None
    eicar_text = "ironbank EICAR payload"
    try:
        service.start()
        client = service.client()

        info = client.get(f"/profiles/{CODE_PROFILE_ID}/plugins/info", timeout=30)
        assert info == {
            "scope": {"kind": "profile", "profile_id": CODE_PROFILE_ID},
            "plugin_count": 4,
            "enabled_count": 2,
        }

        plugins = _plugins_by_id(client)
        broker = plugins["credential_broker"]
        assert broker["stage"] == "preprocess"
        assert broker["version"] == "1"
        assert broker["config"] == {
            "mode": "rewrite",
            "detection_level": "informational",
        }
        assert broker["runtime"]["enabled"] is True
        assert broker["runtime"]["brokered_credentials"] == []
        assert broker["runtime"]["event_count"] == 0
        assert broker["detail_routes"] == [
            {
                "id": "credential_broker_credentials",
                "label": "Credential Broker",
                "kind": "credential_broker",
                "path": f"/profiles/{CODE_PROFILE_ID}/plugins/credential_broker/credentials/info",
            },
            {
                "id": "credential_broker_credentials_reload",
                "label": "Retry Credential Store",
                "kind": "credential_broker",
                "path": f"/profiles/{CODE_PROFILE_ID}/plugins/credential_broker/credentials/reload",
            },
        ]

        sanitizer = plugins["log_sanitizer"]
        assert sanitizer["stage"] == "logging"
        assert sanitizer["runtime"]["enabled"] is True
        assert sanitizer["capabilities"]["credential_sources"] == [
            "security_event.credential_observations"
        ]
        assert sanitizer["detail_routes"] == []

        dummy_pre = plugins["dummy_pre_eicar"]
        assert dummy_pre["stage"] == "preprocess"
        assert dummy_pre["config"]["mode"] == "disable"
        assert dummy_pre["runtime"]["enabled"] is False
        assert dummy_pre["detail_routes"] == []

        default_event = _evaluate(client, eicar_text)
        assert default_event["decision"]["effective"] == "allow"
        assert default_event["file"]["import_content"] == eicar_text
        assert ("rule", "profiles.rules.eicar", None) in _detection_sources(default_event)
        assert ("plugin", None, "dummy_pre_eicar") not in _detection_sources(default_event)
        assert ("plugin", None, "dummy_post_allow") not in _detection_sources(default_event)
        assert all(
            execution["plugin_id"] != "dummy_pre_eicar"
            for execution in default_event["plugin_executions"]
        )

        enabled_pre = client.patch(
            f"/profiles/{CODE_PROFILE_ID}/plugins/dummy_pre_eicar/edit",
            {"mode": "rewrite", "detection_level": "critical"},
            timeout=30,
        )
        assert enabled_pre["id"] == "dummy_pre_eicar"
        assert enabled_pre["config"] == {
            "mode": "rewrite",
            "detection_level": "critical",
        }
        assert enabled_pre["runtime"]["enabled"] is True

        enabled_post = client.patch(
            f"/profiles/{CODE_PROFILE_ID}/plugins/dummy_post_allow/edit",
            {"mode": "allow", "detection_level": "medium"},
            timeout=30,
        )
        assert enabled_post["id"] == "dummy_post_allow"
        assert enabled_post["stage"] == "postprocess"
        assert enabled_post["runtime"]["enabled"] is True

        rewritten_event = _evaluate(client, eicar_text)
        assert rewritten_event["decision"]["effective"] == "allow"
        assert rewritten_event["file"]["import_content"] == "ironbank CAPSEM_REWRITTEN_EICAR payload"
        rewritten_sources = _detection_sources(rewritten_event)
        assert ("rule", "profiles.rules.eicar", None) in rewritten_sources
        assert ("plugin", None, "dummy_pre_eicar") in rewritten_sources
        assert ("plugin", None, "dummy_post_allow") in rewritten_sources
        executions = {item["plugin_id"]: item for item in rewritten_event["plugin_executions"]}
        assert executions["dummy_pre_eicar"]["stage"] == "preprocess"
        assert executions["dummy_pre_eicar"]["applied"] is True
        assert executions["dummy_pre_eicar"]["duration_us"] >= 0
        assert executions["dummy_post_allow"]["stage"] == "postprocess"
        assert executions["dummy_post_allow"]["applied"] is True
        assert executions["dummy_post_allow"]["duration_us"] >= 0
        assert "credential_broker.capture" in rewritten_event["action_trace"]
        assert "credential_broker.substitute" in rewritten_event["action_trace"]

        blocking_pre = client.patch(
            f"/profiles/{CODE_PROFILE_ID}/plugins/dummy_pre_eicar/edit",
            {"mode": "block", "detection_level": "critical"},
            timeout=30,
        )
        assert blocking_pre["runtime"]["enabled"] is True
        blocked_event = _evaluate(client, eicar_text)
        assert blocked_event["decision"]["effective"] == "block"
        blocked_plugin = next(
            detection
            for detection in blocked_event["detections"]
            if detection.get("plugin_id") == "dummy_pre_eicar"
        )
        assert blocked_plugin["detection_level"] == "critical"
        assert blocked_plugin["plugin_mode"] == "block"
        assert blocked_event["file"]["import_content"] == eicar_text

        disabled_pre = client.patch(
            f"/profiles/{CODE_PROFILE_ID}/plugins/dummy_pre_eicar/edit",
            {"mode": "disable"},
            timeout=30,
        )
        assert disabled_pre["runtime"]["enabled"] is False
        after_disable = _evaluate(client, eicar_text)
        assert after_disable["decision"]["effective"] == "allow"
        assert after_disable["file"]["import_content"] == eicar_text
        assert ("plugin", None, "dummy_pre_eicar") not in _detection_sources(after_disable)

        credential_detail = client.get(
            f"/profiles/{CODE_PROFILE_ID}/plugins/credential_broker/credentials/info",
            timeout=30,
        )
        assert credential_detail["scope"] == {"kind": "profile", "profile_id": CODE_PROFILE_ID}
        assert credential_detail["plugin_id"] == "credential_broker"
        assert credential_detail["store"]["ready"] is True
        assert credential_detail["store"]["status"] == "ready"
        assert credential_detail["inventory"] == []
        assert credential_detail["grants"]["profile_enabled"] is True

        reloaded = client.post(
            f"/profiles/{CODE_PROFILE_ID}/plugins/credential_broker/credentials/reload",
            {},
            timeout=30,
        )
        assert reloaded["plugin_id"] == "credential_broker"
        assert reloaded["store"]["ready"] is True
        assert reloaded["store"]["status"] == "ready"
        assert reloaded["inventory"] == []

        unknown = client.patch(
            f"/profiles/{CODE_PROFILE_ID}/plugins/credential_ref/edit",
            {"mode": "rewrite"},
            timeout=30,
        )
        assert unknown["error"] == "unknown plugin: credential_ref"
    finally:
        service.stop()
