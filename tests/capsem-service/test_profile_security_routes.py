"""Profile security route contract.

These routes are the UI/TUI contract for profile-owned enforcement,
detection, plugins, and MCP configuration. They must expose one profile rail:
typed rules, plugin config, and MCP permission mutations. Retired policy,
approval, and plugin-man surfaces must stay burned.
"""

from __future__ import annotations

import json
import subprocess
from typing import Any


PROFILE = "code"
SERVER = "local"


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
    payload = json.loads(raw_body) if raw_body.strip() else None
    return int(status_text), payload


def _seed_mcp_tool_cache(service_env: Any) -> None:
    cache_path = service_env.tmp_dir / "mcp_tool_cache.json"
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


def test_profile_security_routes_expose_single_contract(client: Any, service_env: Any) -> None:
    _seed_mcp_tool_cache(service_env)
    refresh = client.post(f"/profiles/{PROFILE}/mcp/servers/{SERVER}/refresh")
    assert refresh["success"] is True
    assert refresh["server_id"] == SERVER

    enforcement = client.get(f"/profiles/{PROFILE}/enforcement/rules/list")
    detection = client.get(f"/profiles/{PROFILE}/detection/rules/list")
    plugins = client.get(f"/profiles/{PROFILE}/plugins/list")
    mcp_default = client.get(f"/profiles/{PROFILE}/mcp/default/info")
    mcp_tools = client.get(f"/profiles/{PROFILE}/mcp/servers/{SERVER}/tools/list")

    assert enforcement["profile_id"] == PROFILE
    assert all("rule_id" in rule and "action" in rule for rule in enforcement["rules"])
    assert any(rule["default_rule"] for rule in enforcement["rules"])

    assert detection["profile_id"] == PROFILE
    assert all("rule_id" in rule and "detection_level" in rule for rule in detection["rules"])

    assert plugins["scope"] == {"kind": "profile", "profile_id": PROFILE}
    assert plugins["plugins"]
    assert all(plugin["stage"] in {"preprocess", "postprocess", "logging"} for plugin in plugins["plugins"])
    assert all(plugin["config"]["mode"] in {"allow", "ask", "block", "rewrite", "disable"} for plugin in plugins["plugins"])
    assert all("man" not in json.dumps(plugin).lower() for plugin in plugins["plugins"])

    assert mcp_default["action"] in {"allow", "ask", "block"}
    assert mcp_default["rule_id"] == "default.mcp"

    assert isinstance(mcp_tools, list)
    assert {tool["namespaced_name"] for tool in mcp_tools} == {"local__echo", "local__fetch_http"}
    for tool in mcp_tools:
        assert {"namespaced_name", "original_name", "server_name", "permission_action", "permission_source"} <= set(tool)
        assert tool["permission_action"] in {"allow", "ask", "block"}
        assert "approved" not in tool
        assert "policy" not in tool


def test_retired_profile_security_routes_stay_burned(client: Any) -> None:
    for method, path in (
        ("GET", f"/profiles/{PROFILE}/plugins/credential_broker/man"),
        ("GET", f"/profiles/{PROFILE}/mcp/policy"),
        ("GET", "/mcp/policy"),
        ("GET", "/mcp/tools"),
    ):
        status, payload = _status(client, method, path)
        assert status in {404, 405}, (path, status, payload)
