"""Ironbank profile mutation route contract.

These tests use only the public service routes plus the mutation ledger.  The
contract is simple: profile controls mutate profile-owned files, update their
hash pins, and record the exact mutation in ``main.db``.
"""

from __future__ import annotations

import json
import sqlite3
import subprocess
import tomllib
from pathlib import Path
from typing import Any

import blake3
import pytest

from helpers.constants import CODE_PROFILE_ID
from helpers.service import ServiceInstance


pytestmark = pytest.mark.integration


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


def _main_db(service: ServiceInstance) -> Path:
    return service.tmp_dir.parent / "sessions" / "main.db"


def _profile_dir(service: ServiceInstance) -> Path:
    assert service.profiles_dir is not None
    return service.profiles_dir / CODE_PROFILE_ID


def _profile_toml(service: ServiceInstance) -> dict[str, Any]:
    return tomllib.loads((_profile_dir(service) / "profile.toml").read_text())


def _profile_enforcement_text(service: ServiceInstance) -> str:
    return (_profile_dir(service) / "enforcement.toml").read_text()


def _blake3_ref(path: Path) -> str:
    return f"blake3:{blake3.blake3(path.read_bytes()).hexdigest()}"


def _mutation_rows(service: ServiceInstance) -> list[dict[str, Any]]:
    db_path = _main_db(service)
    assert db_path.exists(), f"mutation ledger missing: {db_path}"
    conn = sqlite3.connect(db_path)
    conn.row_factory = sqlite3.Row
    try:
        rows = conn.execute(
            """
            SELECT profile_id, actor, category, filename, affected_path,
                   target_kind, target_key, operation, rule_id,
                   old_hash, new_hash, status, error
              FROM profile_mutation_events
             WHERE profile_id = ?
             ORDER BY id ASC
            """,
            (CODE_PROFILE_ID,),
        ).fetchall()
    finally:
        conn.close()
    return [dict(row) for row in rows]


def _assert_applied(
    row: dict[str, Any], *, category: str, target_kind: str, target_key: str, operation: str
) -> None:
    assert row["profile_id"] == CODE_PROFILE_ID
    assert row["actor"] == "service-api"
    assert row["category"] == category
    assert row["target_kind"] == target_kind
    assert row["target_key"] == target_key
    assert row["operation"] == operation
    assert row["status"] == "applied"
    assert row["error"] is None
    assert row["old_hash"].startswith("blake3:")
    assert row["new_hash"].startswith("blake3:")
    assert row["old_hash"] != row["new_hash"]


def test_profile_mutation_routes_persist_profile_files_hashes_and_ledger() -> None:
    service = ServiceInstance()
    try:
        service.start()
        client = service.client()

        enforcement_rule = {
            "name": "ironbank_http_block",
            "action": "block",
            "match": 'http.host == "ironbank-block.example"',
            "detection_level": "high",
            "reason": "Ironbank proof that enforcement edits persist.",
        }
        enforcement = client.put(
            f"/profiles/{CODE_PROFILE_ID}/enforcement/rules/ironbank_http_block/edit",
            enforcement_rule,
            timeout=30,
        )
        assert enforcement["rule_id"] == "ironbank_http_block"
        assert enforcement["compiled_rule_id"] == "profiles.rules.ironbank_http_block"
        assert enforcement["rule"]["action"] == "block"
        assert "ironbank_http_block" in _profile_enforcement_text(service)
        assert _profile_toml(service)["files"]["enforcement"]["hash"] == _blake3_ref(
            _profile_dir(service) / "enforcement.toml"
        )

        detection_rule = {
            "name": "ironbank_dns_detect",
            "action": "allow",
            "match": 'dns.qname.contains("ironbank.example")',
            "detection_level": "informational",
            "reason": "Ironbank proof that detection edits persist.",
        }
        detection = client.put(
            f"/profiles/{CODE_PROFILE_ID}/detection/rules/ironbank_dns_detect/edit",
            detection_rule,
            timeout=30,
        )
        assert detection["rule_id"] == "ironbank_dns_detect"
        assert detection["rule"]["detection_level"] == "informational"
        assert "ironbank_dns_detect" in _profile_enforcement_text(service)
        assert _profile_toml(service)["files"]["enforcement"]["hash"] == _blake3_ref(
            _profile_dir(service) / "enforcement.toml"
        )

        mcp_default = client.patch(
            f"/profiles/{CODE_PROFILE_ID}/mcp/default/edit",
            {"action": "ask"},
            timeout=30,
        )
        assert mcp_default["profile_id"] == CODE_PROFILE_ID
        assert mcp_default["action"] == "ask"
        assert mcp_default["mutation"]["target_kind"] == "mcp_default"
        assert client.get(f"/profiles/{CODE_PROFILE_ID}/mcp/default/info")["action"] == "ask"

        mcp_tool = client.patch(
            f"/profiles/{CODE_PROFILE_ID}/mcp/servers/capsem/tools/snapshot/edit",
            {"action": "block"},
            timeout=30,
        )
        assert mcp_tool["profile_id"] == CODE_PROFILE_ID
        assert mcp_tool["server_id"] == "capsem"
        assert mcp_tool["tool_id"] == "snapshot"
        assert mcp_tool["action"] == "block"
        assert "mcp_capsem_snapshot_permission" in _profile_enforcement_text(service)

        mcp_server = client.put(
            f"/profiles/{CODE_PROFILE_ID}/mcp/servers/ironbank/edit",
            {"url": "https://mcp.ironbank.invalid/server", "enabled": False},
            timeout=30,
        )
        assert mcp_server["profile_id"] == CODE_PROFILE_ID
        assert mcp_server["server_id"] == "ironbank"
        assert mcp_server["enabled"] is False
        assert any(
            server["name"] == "ironbank" and server["enabled"] is False
            for server in client.get(f"/profiles/{CODE_PROFILE_ID}/mcp/servers/list")
        )

        plugin = client.patch(
            f"/profiles/{CODE_PROFILE_ID}/plugins/dummy_pre_eicar/edit",
            {"mode": "rewrite", "detection_level": "critical"},
            timeout=30,
        )
        assert plugin["id"] == "dummy_pre_eicar"
        assert plugin["config"] == {"mode": "rewrite", "detection_level": "critical"}
        assert _profile_toml(service)["plugins"]["dummy_pre_eicar"] == {
            "mode": "rewrite",
            "detection_level": "critical",
        }

        deleted = client.delete(
            f"/profiles/{CODE_PROFILE_ID}/enforcement/rules/ironbank_http_block/delete",
            timeout=30,
        )
        assert deleted == {"rule_id": "ironbank_http_block", "deleted": True}
        assert "ironbank_http_block" not in _profile_enforcement_text(service)

        status, rejected = _status(
            client,
            "PATCH",
            f"/profiles/{CODE_PROFILE_ID}/plugins/dummy_pre_eicar/edit",
            {"mode": "rewrite", "fallback": True},
        )
        assert status == 422
        assert "unknown field" in rejected

        # write(event).await accepts into the logger-owned buffer. Graceful
        # service shutdown is the visibility barrier before opening main.db.
        service.stop()
        rows = _mutation_rows(service)
        observed = {
            (row["category"], row["target_kind"], row["target_key"], row["operation"])
            for row in rows
        }
        assert {
            ("enforcement", "security_rule", "ironbank_http_block", "upsert"),
            ("enforcement", "security_rule", "ironbank_dns_detect", "upsert"),
            ("mcp", "mcp_default", "default.mcp", "permission"),
            ("mcp", "mcp_tool", "capsem/snapshot", "permission"),
            ("mcp", "mcp_server", "ironbank", "upsert"),
            ("plugin", "plugin", "dummy_pre_eicar", "edit"),
            ("enforcement", "security_rule", "ironbank_http_block", "delete"),
        } <= observed

        rows_by_key = {
            (row["category"], row["target_kind"], row["target_key"], row["operation"]): row
            for row in rows
        }
        _assert_applied(
            rows_by_key[("plugin", "plugin", "dummy_pre_eicar", "edit")],
            category="plugin",
            target_kind="plugin",
            target_key="dummy_pre_eicar",
            operation="edit",
        )
        assert (
            rows_by_key[("plugin", "plugin", "dummy_pre_eicar", "edit")]["filename"]
            == "profile.toml"
        )
        assert (
            rows_by_key[("plugin", "plugin", "dummy_pre_eicar", "edit")]["affected_path"]
            == "profiles/code/profile.toml"
        )

    finally:
        service.stop()
