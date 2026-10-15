"""Settings-scope security route contract.

These routes are the UI/TUI contract for plugins and MCP configuration,
answered from the built-in defaults, settings.toml and the corp config. Retired
policy, approval, and plugin-man surfaces must stay burned.
"""

from __future__ import annotations

import json
from typing import Any

from helpers.service import ServiceInstance

SERVER = "local"


def _status(client: Any, method: str, path: str, body: dict | None = None) -> tuple[int, Any]:
    return client.call_json(method, path, body, timeout=30)


def _seed_mcp_tool_cache(service_env: Any) -> None:
    cache_path = service_env.home_dir / "mcp_tool_cache.json"
    cache_path.write_text(
        json.dumps(
            [
                {
                    "namespaced_name": "local__echo",
                    "original_name": "echo",
                    "description": "Echo",
                    "server_name": SERVER,
                    "annotations": None,
                    "pin_hash": "echo-pin",
                    "first_seen": "2026-06-10T00:00:00Z",
                    "last_seen": "2026-06-10T00:00:00Z",
                    "approved": True,
                },
                {
                    "namespaced_name": "local__fetch_http",
                    "original_name": "fetch_http",
                    "description": "Fetch HTTP",
                    "server_name": SERVER,
                    "annotations": None,
                    "pin_hash": "test-pin",
                    "first_seen": "2026-06-10T00:00:00Z",
                    "last_seen": "2026-06-10T00:00:00Z",
                    "approved": True,
                }
            ]
        )
    )


def test_settings_security_routes_expose_single_contract() -> None:
    service = ServiceInstance()
    _seed_mcp_tool_cache(service)
    service.start()
    try:
        client = service.client()
        refresh = client.post(f"/mcp/servers/{SERVER}/refresh")
        assert refresh["success"] is True
        assert refresh["server_id"] == SERVER

        plugins = client.get("/plugins/list")
        mcp_info = client.get("/mcp/info")
        mcp_default = client.get("/mcp/default/info")
        mcp_tools = client.get(f"/mcp/servers/{SERVER}/tools/list")
    finally:
        service.stop()

    assert set(plugins) == {"plugins"}
    assert {plugin["id"] for plugin in plugins["plugins"]} == {
        "credential_broker",
        "dummy_post_allow",
        "dummy_pre_eicar",
        "log_sanitizer",
    }
    assert all(plugin["name"] and plugin["description"] for plugin in plugins["plugins"])
    assert all("scope" not in plugin for plugin in plugins["plugins"])
    assert all(plugin["stage"] in {"preprocess", "postprocess", "logging"} for plugin in plugins["plugins"])
    assert all(plugin["config"]["mode"] in {"allow", "ask", "block", "rewrite", "disable"} for plugin in plugins["plugins"])
    assert all("man" not in json.dumps(plugin).lower() for plugin in plugins["plugins"])

    assert mcp_info["builtin_local_enabled"] is True
    assert mcp_info["server_count"] == mcp_info["manual_server_count"] + 1

    assert mcp_default["action"] in {"allow", "ask", "block"}
    assert mcp_default["rule_id"] == "default.mcp"
    assert mcp_default["source"] in {"corp", "settings", "default"}

    assert isinstance(mcp_tools, list)
    assert {tool["namespaced_name"] for tool in mcp_tools} == {"local__echo", "local__fetch_http"}
    for tool in mcp_tools:
        assert {"namespaced_name", "original_name", "server_name", "permission_action", "permission_source"} <= set(tool)
        assert tool["permission_action"] in {"allow", "ask", "block"}
        assert tool["permission_source"] in {"corp", "settings", "default"}
        assert "approved" not in tool
        assert "policy" not in tool


def test_retired_security_routes_stay_burned(client: Any) -> None:
    for method, path in (
        ("GET", "/plugins/info"),
        ("GET", "/plugins/credential_broker/man"),
        ("PUT", "/mcp/servers/local/edit"),
        ("DELETE", "/mcp/servers/local/delete"),
        ("GET", "/mcp/policy"),
        ("GET", "/mcp/tools"),
    ):
        status, payload = _status(client, method, path)
        assert status in {404, 405}, (path, status, payload)
