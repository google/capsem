"""Service-facing security rule contract tests."""

from __future__ import annotations

import pytest

from helpers.constants import CODE_PROFILE_ID
from helpers.service import ServiceInstance


pytestmark = pytest.mark.integration


def test_rule_routes_reject_old_policy_authoring_and_evaluate_security_event_shape() -> None:
    service = ServiceInstance()
    try:
        service.start()
        client = service.client()

        old_table = "policy" + ".http.block_old"
        rule_payload = {
            "rules_toml": """
[__OLD_TABLE__]
name = "block_old"
action = "block"
match = 'http.host == "evil.example"'
""".replace("__OLD_TABLE__", old_table).strip(),
            "event": {
                "event_type": "http.request",
                "http_host": "evil.example",
            },
        }
        rejected = client.post(
            f"/profiles/{CODE_PROFILE_ID}/enforcement/evaluate",
            rule_payload,
            timeout=30,
        )
        assert "error" in rejected
        assert old_table in rejected["error"]

        modern_payload = {
            "rules_toml": """
[profiles.rules.block_evil]
name = "block_evil"
action = "block"
detection_level = "high"
match = 'http.host == "evil.example"'
""".strip(),
            "event": {
                "event_type": "http.request",
                "http_host": "evil.example",
            },
        }
        evaluated = client.post(
            f"/profiles/{CODE_PROFILE_ID}/enforcement/evaluate",
            modern_payload,
            timeout=30,
        )
        assert set(evaluated) == {"event"}
        event = evaluated["event"]
        assert event["event_type"] == "http.request"
        assert event["http"]["host"] == "evil.example"
        assert event["decision"]["effective"] == "block"
        detections = event["detections"]
        assert len(detections) == 1
        assert detections[0]["source"] == "rule"
        assert detections[0]["rule_id"] == "profiles.rules.block_evil"
        assert detections[0]["detection_level"] == "high"
    finally:
        service.stop()
