"""Profile plugin route contract.

Plugin configuration is profile-owned and exposed through UDS routes. This
test keeps the UI/TUI contract honest without reaching into product internals:
typed stages, enum modes, route-owned credential broker details, mutation, and
unknown-plugin rejection all have to work through the same public surface.
"""

from __future__ import annotations

import json
import subprocess
from typing import Any


PROFILE = "code"
PLUGIN_IDS = {
    "credential_broker",
    "log_sanitizer",
    "dummy_pre_eicar",
    "dummy_post_allow",
}
PLUGIN_STAGES = {"preprocess", "postprocess", "logging"}
PLUGIN_MODES = {"allow", "ask", "block", "rewrite", "disable"}
DETECTION_LEVELS = {"none", "informational", "low", "medium", "high", "critical"}


def _status(client: Any, method: str, path: str, body: dict | None = None) -> tuple[int, Any]:
    cmd = [
        "curl",
        "-s",
        "-S",
        "--unix-socket",
        client.socket_path,
        "-X",
        method,
        "-H",
        "Content-Type: application/json",
        "-w",
        "\n%{http_code}",
        "--max-time",
        "30",
        f"http://localhost{path}",
    ]
    if body is not None:
        cmd.extend(["-d", json.dumps(body)])
    result = subprocess.run(cmd, capture_output=True, text=True, timeout=35)
    assert result.returncode == 0, (path, result.stderr)
    raw_body, _, status_text = result.stdout.rpartition("\n")
    if raw_body.strip():
        try:
            payload = json.loads(raw_body)
        except json.JSONDecodeError:
            payload = raw_body
    else:
        payload = None
    return int(status_text), payload


def _plugins_by_id(client: Any) -> dict[str, dict]:
    response = client.get(f"/profiles/{PROFILE}/plugins/list")
    assert response["scope"] == {"kind": "profile", "profile_id": PROFILE}
    assert set(response) == {"scope", "plugins"}
    plugins = {plugin["id"]: plugin for plugin in response["plugins"]}
    assert set(plugins) == PLUGIN_IDS
    return plugins


def _assert_plugin_contract(plugin: dict, *, plugin_id: str, stage: str) -> None:
    assert plugin["id"] == plugin_id
    assert plugin["name"]
    assert plugin["description"]
    assert plugin["version"] == "1"
    assert plugin["stage"] == stage
    assert plugin["stage"] in PLUGIN_STAGES
    assert plugin["scope"] == {"kind": "profile", "profile_id": PROFILE}
    assert plugin["config"]["mode"] in PLUGIN_MODES
    assert plugin["default_config"]["mode"] in PLUGIN_MODES
    assert plugin["config"]["detection_level"] in DETECTION_LEVELS
    assert plugin["default_config"]["detection_level"] in DETECTION_LEVELS
    assert isinstance(plugin["overridden"], bool)

    runtime = plugin["runtime"]
    assert runtime["enabled"] == (plugin["config"]["mode"] != "disable")
    for counter in (
        "event_count",
        "execution_count",
        "applied_count",
        "skipped_count",
        "total_duration_us",
        "max_duration_us",
        "detection_count",
        "block_count",
        "rewrite_count",
    ):
        assert isinstance(runtime[counter], int), (plugin_id, counter, runtime[counter])
        assert runtime[counter] >= 0
    assert runtime["last_error"] is None or isinstance(runtime["last_error"], str)
    assert isinstance(runtime["brokered_credentials"], list)

    capabilities = plugin["capabilities"]
    assert isinstance(capabilities["event_families"], list)
    assert isinstance(capabilities["credential_providers"], list)
    assert isinstance(capabilities["credential_sources"], list)
    assert "man" not in json.dumps(plugin).lower()


def test_profile_plugin_routes_expose_typed_stage_contract(client: Any) -> None:
    info = client.get(f"/profiles/{PROFILE}/plugins/info")
    assert info == {
        "scope": {"kind": "profile", "profile_id": PROFILE},
        "plugin_count": 4,
        "enabled_count": 2,
    }

    plugins = _plugins_by_id(client)
    _assert_plugin_contract(plugins["credential_broker"], plugin_id="credential_broker", stage="preprocess")
    _assert_plugin_contract(plugins["log_sanitizer"], plugin_id="log_sanitizer", stage="logging")
    _assert_plugin_contract(plugins["dummy_pre_eicar"], plugin_id="dummy_pre_eicar", stage="preprocess")
    _assert_plugin_contract(plugins["dummy_post_allow"], plugin_id="dummy_post_allow", stage="postprocess")

    assert plugins["credential_broker"]["config"] == {
        "mode": "rewrite",
        "detection_level": "informational",
    }
    assert plugins["log_sanitizer"]["config"] == {
        "mode": "rewrite",
        "detection_level": "informational",
    }
    assert plugins["dummy_pre_eicar"]["config"]["mode"] == "disable"
    assert plugins["dummy_post_allow"]["config"]["mode"] == "disable"
    assert plugins["dummy_pre_eicar"]["runtime"]["enabled"] is False
    assert plugins["dummy_post_allow"]["runtime"]["enabled"] is False

    broker_routes = plugins["credential_broker"]["detail_routes"]
    assert broker_routes == [
        {
            "id": "credential_broker_credentials",
            "label": "Credential Broker",
            "kind": "credential_broker",
            "path": f"/profiles/{PROFILE}/plugins/credential_broker/credentials/info",
        },
        {
            "id": "credential_broker_credentials_reload",
            "label": "Retry Credential Store",
            "kind": "credential_broker",
            "path": f"/profiles/{PROFILE}/plugins/credential_broker/credentials/reload",
        },
    ]
    assert plugins["log_sanitizer"]["detail_routes"] == []
    assert plugins["dummy_pre_eicar"]["detail_routes"] == []
    assert plugins["dummy_post_allow"]["detail_routes"] == []

    broker_detail = client.get(f"/profiles/{PROFILE}/plugins/credential_broker/info")
    assert broker_detail == plugins["credential_broker"]


def test_profile_plugin_routes_mutate_only_known_enum_contract(client: Any) -> None:
    enabled = client.patch(
        f"/profiles/{PROFILE}/plugins/dummy_pre_eicar/edit",
        {"mode": "block", "detection_level": "critical"},
    )
    assert enabled["id"] == "dummy_pre_eicar"
    assert enabled["stage"] == "preprocess"
    assert enabled["overridden"] is True
    assert enabled["config"] == {"mode": "block", "detection_level": "critical"}
    assert enabled["runtime"]["enabled"] is True

    listed = _plugins_by_id(client)["dummy_pre_eicar"]
    assert listed["config"] == enabled["config"]
    assert listed["runtime"]["enabled"] is True

    disabled = client.patch(
        f"/profiles/{PROFILE}/plugins/dummy_pre_eicar/edit",
        {"mode": "disable"},
    )
    assert disabled["id"] == "dummy_pre_eicar"
    assert disabled["config"]["mode"] == "disable"
    assert disabled["runtime"]["enabled"] is False

    status, payload = _status(
        client,
        "PATCH",
        f"/profiles/{PROFILE}/plugins/dummy_pre_eicar/edit",
        {"mode": "inspect"},
    )
    assert status == 422
    assert "unknown variant" in payload

    status, payload = _status(
        client,
        "PATCH",
        f"/profiles/{PROFILE}/plugins/dummy_pre_eicar/edit",
        {"mode": "rewrite", "fallback": True},
    )
    assert status == 422
    assert "unknown field" in payload

    status, payload = _status(
        client,
        "PATCH",
        f"/profiles/{PROFILE}/plugins/credential_ref/edit",
        {"mode": "rewrite"},
    )
    assert status == 404
    assert payload == {"error": "unknown plugin: credential_ref"}


def test_credential_broker_detail_and_reload_routes_share_one_contract(client: Any) -> None:
    detail = client.get(f"/profiles/{PROFILE}/plugins/credential_broker/credentials/info")
    assert detail["scope"] == {"kind": "profile", "profile_id": PROFILE}
    assert detail["plugin_id"] == "credential_broker"
    assert set(detail) == {
        "scope",
        "plugin_id",
        "store",
        "inventory",
        "grants",
        "corp_constraints",
    }
    assert detail["store"]["ready"] is True
    assert detail["store"]["status"] == "ready"
    assert detail["inventory"] == []
    assert detail["grants"] == {
        "profile_enabled": True,
        "vm_grants": [],
        "fork_default": "inherit_profile",
    }
    assert detail["corp_constraints"] == []

    reloaded = client.post(
        f"/profiles/{PROFILE}/plugins/credential_broker/credentials/reload",
        {},
    )
    assert reloaded["scope"] == detail["scope"]
    assert reloaded["plugin_id"] == "credential_broker"
    assert reloaded["inventory"] == []
    assert reloaded["grants"] == detail["grants"]
    assert reloaded["corp_constraints"] == []
    assert reloaded["store"]["ready"] is True
    assert reloaded["store"]["status"] == "ready"
    assert reloaded["store"]["backend"] == detail["store"]["backend"]
