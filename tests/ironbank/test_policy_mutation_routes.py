"""Ironbank policy mutation route contract.

These tests use only the public service routes, the user's settings.toml and
the host mutation ledger. The contract: plugin and MCP permission controls
edit settings.toml, a value the corp config sets is refused, and every applied
edit is recorded exactly in `policy_mutation_events`.
"""

from __future__ import annotations

import sqlite3
import tomllib
from typing import Any

import blake3
import pytest
from helpers.service import ServiceInstance

pytestmark = pytest.mark.integration

LEDGER_COLUMNS = (
    "id",
    "timestamp_unix_ms",
    "mutation_id",
    "actor",
    "category",
    "filename",
    "affected_path",
    "target_kind",
    "target_key",
    "operation",
    "rule_id",
    "old_hash",
    "old_size",
    "new_hash",
    "new_size",
    "status",
    "error",
    "trace_id",
)


def _status(client: Any, method: str, path: str, body: dict | None = None) -> tuple[int, Any]:
    return client.call_json(method, path, body, timeout=30)


def _settings(service: ServiceInstance) -> dict[str, Any]:
    return tomllib.loads((service.home_dir / "settings.toml").read_text())


def _mutation_rows(service: ServiceInstance) -> list[dict[str, Any]]:
    db_path = service.home_dir / "sessions" / "host.db"
    assert db_path.exists(), f"mutation ledger missing: {db_path}"
    conn = sqlite3.connect(db_path)
    conn.row_factory = sqlite3.Row
    try:
        columns = tuple(
            row["name"] for row in conn.execute("PRAGMA table_info(policy_mutation_events)")
        )
        assert columns == LEDGER_COLUMNS
        assert not conn.execute(
            "SELECT name FROM sqlite_master WHERE name = 'profile_mutation_events'"
        ).fetchall()
        rows = conn.execute("SELECT * FROM policy_mutation_events ORDER BY id ASC").fetchall()
    finally:
        conn.close()
    return [dict(row) for row in rows]


def _assert_applied(row: dict[str, Any], *, category: str, target_kind: str, target_key: str,
                    operation: str, rule_id: str | None) -> None:
    assert row["actor"] == "service-api"
    assert row["category"] == category
    assert row["target_kind"] == target_kind
    assert row["target_key"] == target_key
    assert row["operation"] == operation
    assert row["rule_id"] == rule_id
    assert row["filename"] == "settings.toml"
    assert row["affected_path"] == "settings.toml"
    assert row["status"] == "applied"
    assert row["error"] is None
    assert len(row["mutation_id"]) == 12
    assert row["old_hash"].startswith("blake3:") and len(row["old_hash"]) == 71
    assert row["new_hash"].startswith("blake3:") and len(row["new_hash"]) == 71
    assert row["old_hash"] != row["new_hash"]
    assert row["new_size"] > 0


def test_policy_mutation_routes_edit_settings_and_record_the_ledger() -> None:
    service = ServiceInstance()
    # The corp config owns this plugin; a settings edit of it is refused.
    (service.home_dir / "corp.toml").write_text(
        '[plugins.log_sanitizer]\nmode = "rewrite"\ndetection_level = "informational"\n'
    )
    try:
        service.start()
        client = service.client()

        mcp_default = client.patch("/mcp/default/edit", {"action": "ask"}, timeout=30)
        assert set(mcp_default) == {"action", "mutation"}
        assert mcp_default["action"] == "ask"
        assert mcp_default["mutation"]["target_kind"] == "mcp_default"
        assert client.get("/mcp/default/info") == {
            "action": "ask",
            "source": "settings",
            "rule_id": "default.mcp",
        }
        assert _settings(service)["default"]["mcp"]["action"] == "ask"

        mcp_tool = client.patch("/mcp/servers/local/tools/probe/edit", {"action": "block"}, timeout=30)
        assert mcp_tool["server_id"] == "local"
        assert mcp_tool["tool_id"] == "probe"
        assert mcp_tool["action"] == "block"
        tool_rule_id = mcp_tool["mutation"]["rule_id"]
        assert tool_rule_id.startswith("profiles.rules.")
        tool_rule = _settings(service)["profiles"]["rules"][tool_rule_id.removeprefix("profiles.rules.")]
        assert tool_rule["action"] == "block"
        assert tool_rule["match"] == 'mcp.server.name == "local" && mcp.tool_call.name == "probe"'

        plugin = client.patch(
            "/plugins/dummy_pre_eicar/edit",
            {"mode": "rewrite", "detection_level": "critical"},
            timeout=30,
        )
        assert plugin["id"] == "dummy_pre_eicar"
        assert plugin["overridden"] is True
        assert plugin["config"] == {"mode": "rewrite", "detection_level": "critical"}
        assert _settings(service)["plugins"]["dummy_pre_eicar"] == {
            "mode": "rewrite",
            "detection_level": "critical",
        }

        before_refusals = (service.home_dir / "settings.toml").read_bytes()
        status, refused = _status(client, "PATCH", "/plugins/log_sanitizer/edit", {"mode": "disable"})
        assert status == 400, (status, refused)
        assert refused == {"error": "plugin log_sanitizer is set by the corp config"}

        status, rejected = _status(
            client, "PATCH", "/plugins/dummy_pre_eicar/edit", {"mode": "rewrite", "fallback": True}
        )
        assert status == 422
        assert "unknown field" in rejected

        status, unconfigured = _status(
            client, "PATCH", "/mcp/servers/not-configured/tools/probe/edit", {"action": "block"}
        )
        assert status == 400, (status, unconfigured)
        assert (service.home_dir / "settings.toml").read_bytes() == before_refusals

        settings_hash = f"blake3:{blake3.blake3(before_refusals).hexdigest()}"

        # write(event).await accepts into the logger-owned buffer. Graceful
        # service shutdown is the visibility barrier before opening the host ledger.
        service.stop(cleanup=False)
        rows = _mutation_rows(service)
        assert [
            (row["category"], row["target_kind"], row["target_key"], row["operation"])
            for row in rows
        ] == [
            ("mcp", "mcp_default", "default.mcp", "permission"),
            ("mcp", "mcp_tool", "local/probe", "permission"),
            ("plugin", "plugin", "dummy_pre_eicar", "edit"),
        ]
        _assert_applied(rows[0], category="mcp", target_kind="mcp_default", target_key="default.mcp",
                        operation="permission", rule_id="default.mcp")
        _assert_applied(rows[1], category="mcp", target_kind="mcp_tool", target_key="local/probe",
                        operation="permission", rule_id=tool_rule_id)
        _assert_applied(rows[2], category="plugin", target_kind="plugin", target_key="dummy_pre_eicar",
                        operation="edit", rule_id=None)
        # Each edit starts from the file the previous one wrote.
        assert rows[1]["old_hash"] == rows[0]["new_hash"]
        assert rows[2]["old_hash"] == rows[1]["new_hash"]
        assert rows[2]["new_hash"] == settings_hash
        assert rows[2]["new_size"] == len(before_refusals)
    finally:
        service.stop()


def test_retired_profile_mutation_routes_are_gone() -> None:
    service = ServiceInstance()
    try:
        service.start()
        client = service.client()
        for method, path, body in (
            ("PUT", "/profiles/code/enforcement/rules/probe/edit", {"action": "block"}),
            ("DELETE", "/profiles/code/enforcement/rules/probe/delete", None),
            ("PUT", "/profiles/code/detection/rules/probe/edit", {"action": "allow"}),
            ("PUT", "/profiles/code/mcp/servers/probe/edit", {"enabled": False}),
            ("DELETE", "/profiles/code/mcp/servers/probe/delete", None),
            ("POST", "/profiles/code/skills/add", {}),
            ("PATCH", "/profiles/code/plugins/dummy_pre_eicar/edit", {"mode": "block"}),
        ):
            status, payload = _status(client, method, path, body)
            assert status in {404, 405}, (method, path, status, payload)
    finally:
        service.stop()
