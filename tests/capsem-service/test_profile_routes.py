"""Profile route contract for service-facing profile truth.

These tests prove the profile page can be built from route-owned facts: the
route supplies profile names, descriptions, icons, surfaces, assets, rules,
detections, plugins, and MCP state. The frontend must not invent them.
"""

from __future__ import annotations


def _profiles_by_id(payload: dict) -> dict[str, dict]:
    profiles = payload.get("profiles")
    assert isinstance(profiles, list), payload
    return {profile["id"]: profile for profile in profiles}


def _assert_profile_summary(profile: dict, *, profile_id: str, name: str) -> None:
    assert profile["id"] == profile_id
    assert profile["name"] == name
    assert isinstance(profile["description"], str) and profile["description"]
    assert set(profile["availability"]) == {"web", "shell", "mobile"}
    assert all(isinstance(value, bool) for value in profile["availability"].values())
    assert profile["source"] in {"profile", "built_in"}
    assert isinstance(profile["rule_count"], int) and profile["rule_count"] > 0
    assert isinstance(profile["default_rule_count"], int)
    assert isinstance(profile["plugin_count"], int) and profile["plugin_count"] > 0
    assert isinstance(profile["mcp_server_count"], int)
    assert profile["update_semantics"] == {
        "new_sessions": "use_current_profile_catalog",
        "existing_vms": "pinned_until_recreate",
        "upgrade_action": "recreate_vm",
    }
    assert "enabled_by" not in profile
    assert "policy" not in profile


def test_profiles_list_and_status_expose_profile_owned_contract(client):
    listed = _profiles_by_id(client.get("/profiles/list"))

    assert {"code", "co-work"} <= listed.keys()
    _assert_profile_summary(listed["code"], profile_id="code", name="Code")
    _assert_profile_summary(listed["co-work"], profile_id="co-work", name="Co-work")
    assert listed["code"]["description"] == "Optimized for coding and long-running agents."
    assert listed["co-work"]["description"] == "Shared profile for collaborative agent sessions."

    status = client.get("/profiles/status")
    assert "asset_manifest" in status
    assert status["profile_count"] >= 2
    assert status["ready_count"] >= 0
    status_by_id = {profile["id"]: profile for profile in status["profiles"]}
    assert {"code", "co-work"} <= status_by_id.keys()
    for profile_id, profile_status in status_by_id.items():
        assert "ready" in profile_status
        assert isinstance(profile_status["asset_count"], int)
        assert profile_status["update_semantics"] == {
            "new_sessions": "use_current_profile_catalog",
            "existing_vms": "pinned_until_recreate",
            "upgrade_action": "recreate_vm",
        }
        assert "missing_assets" in profile_status
        assert "invalid_assets" in profile_status
        assert profile_status["id"] == profile_id


def test_profile_info_routes_expose_assets_rules_plugins_mcp_and_detection(client):
    for profile_id in ("code", "co-work"):
        info = client.get(f"/profiles/{profile_id}/info")
        profile = info["profile"]
        _assert_profile_summary(
            profile,
            profile_id=profile_id,
            name="Code" if profile_id == "code" else "Co-work",
        )
        assert "obom" in info

        assets = client.get(f"/profiles/{profile_id}/assets/status")
        assert assets["profile_id"] == profile_id
        assert isinstance(assets["assets"], list)
        assert "manifest" in assets
        assert "filesystem" not in assets
        assert "compression" not in assets

        enforcement = client.get(f"/profiles/{profile_id}/enforcement/rules/list")
        assert enforcement["profile_id"] == profile_id
        assert isinstance(enforcement["rules"], list)
        assert any(rule["default_rule"] for rule in enforcement["rules"])
        assert all(rule["rule_id"] and rule["name"] for rule in enforcement["rules"])

        detection = client.get(f"/profiles/{profile_id}/detection/rules/list")
        assert detection["profile_id"] == profile_id
        assert isinstance(detection["rules"], list)

        plugins = client.get(f"/profiles/{profile_id}/plugins/list")
        assert plugins["scope"] == {"kind": "profile", "profile_id": profile_id}
        assert plugins["plugins"]
        assert all(plugin["name"] and plugin["description"] for plugin in plugins["plugins"])
        assert all(
            plugin["stage"] in {"preprocess", "postprocess", "logging"}
            for plugin in plugins["plugins"]
        )

        mcp = client.get(f"/profiles/{profile_id}/mcp/info")
        assert mcp["profile_id"] == profile_id
        assert isinstance(mcp["server_count"], int)
        assert isinstance(mcp["builtin_local_enabled"], bool)
