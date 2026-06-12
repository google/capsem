"""Ironbank black-box capsem-doctor ledger tests."""

from __future__ import annotations

import json
import re
import shlex
import sqlite3
from pathlib import Path

import pytest

from helpers.constants import CODE_PROFILE_ID, DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.mock_server import MOCK_SERVER_BINARY, start_mock_server, stop_process
from helpers.service import ServiceInstance, wait_exec_ready, vm_name


PROJECT_ROOT = Path(__file__).resolve().parents[2]
ASSETS_DIR = PROJECT_ROOT / "assets"
PROFILES_DIR = PROJECT_ROOT / "target" / "config" / "profiles"

pytestmark = pytest.mark.integration

EXPECTED_SUBSTITUTION_COLUMNS = {
    "id",
    "event_id",
    "timestamp",
    "material_class",
    "source",
    "event_type",
    "algorithm",
    "substitution_ref",
    "outcome",
    "provider",
    "confidence",
    "trace_id",
    "context_json",
}

EXPECTED_SECURITY_LATEST_FIELDS = {
    "timestamp_unix_ms",
    "event_id",
    "event_type",
    "rule_id",
    "rule_action",
    "detection_level",
    "rule_json",
    "event_json",
    "trace_id",
}

BROKER_OUTCOMES = {"captured", "brokered", "injected", "error"}
HAPPY_PATH_BROKER_OUTCOMES = {"captured", "brokered", "injected"}
RAW_SECRET_MARKERS = {
    "capsem_test_openai_api_key",
    "capsem_test_api_key",
    "capsem_test_oauth_access",
    "capsem_test_oauth_refresh",
    "capsem_test_oauth_id",
    "capsem_test_oauth_code",
    "capsem_test_oauth_client_secret",
}


def _connect_session_db(session_root: Path, session_id: str) -> sqlite3.Connection:
    db_path = session_root / session_id / "session.db"
    assert db_path.exists(), f"session DB missing at {db_path}"
    conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    conn.row_factory = sqlite3.Row
    return conn


def _table_columns(conn: sqlite3.Connection, table: str) -> set[str]:
    return {row[1] for row in conn.execute(f"PRAGMA table_info({table})").fetchall()}


def _single(conn: sqlite3.Connection, query: str, params: tuple = ()) -> sqlite3.Row:
    row = conn.execute(query, params).fetchone()
    assert row is not None, f"expected row for query: {query}"
    return row


def _count(conn: sqlite3.Connection, table: str, where: str = "1 = 1") -> int:
    return int(conn.execute(f"SELECT COUNT(*) FROM {table} WHERE {where}").fetchone()[0])


def _assert_ledger_id(value: object) -> None:
    assert isinstance(value, str)
    assert re.fullmatch(r"[0-9a-f]{12}", value), value


def _assert_no_raw_secret_markers_in_session_db(conn: sqlite3.Connection) -> None:
    tables = [
        row[0]
        for row in conn.execute(
            "SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name"
        ).fetchall()
    ]
    for table in tables:
        columns = conn.execute(f"PRAGMA table_info({table})").fetchall()
        text_columns = [row[1] for row in columns if str(row[2]).upper() in {"TEXT", ""}]
        if not text_columns:
            continue
        selected = ", ".join(f'"{column}"' for column in text_columns)
        for row in conn.execute(f'SELECT {selected} FROM "{table}"').fetchall():
            for column, value in zip(text_columns, row, strict=True):
                if not isinstance(value, str):
                    continue
                leaked = [marker for marker in RAW_SECRET_MARKERS if marker in value]
                assert not leaked, f"raw secret marker leaked in {table}.{column}: {leaked}"


def test_capsem_doctor_pays_protocol_and_security_ledger_debt():
    assert MOCK_SERVER_BINARY.exists(), f"{MOCK_SERVER_BINARY} missing; restore mock server runtime"
    assert ASSETS_DIR.exists(), f"{ASSETS_DIR} missing; build VM assets before Ironbank"
    assert PROFILES_DIR.exists(), f"{PROFILES_DIR} missing; materialize profile config before Ironbank"

    service = ServiceInstance()
    client = None
    mock_proc = None
    session_id = vm_name("ironbank-doctor")
    try:
        service.start()
        client = service.client()
        mock_proc, ready = start_mock_server()
        mock_base_url = ready["base_url"]
        create = client.post(
            "/vms/create",
            {
                "name": session_id,
                "profile_id": CODE_PROFILE_ID,
                "ram_mb": DEFAULT_RAM_MB,
                "cpus": DEFAULT_CPUS,
                "env": {"CAPSEM_MOCK_SERVER_BASE_URL": mock_base_url},
            },
            timeout=90,
        )
        assert create is not None
        assert create.get("id") == session_id or create.get("name") == session_id
        assert wait_exec_ready(client, session_id, timeout=EXEC_READY_TIMEOUT)

        exec_resp = client.post(
            f"/vms/{session_id}/exec",
            {
                "command": (
                    "export CAPSEM_MOCK_SERVER_BASE_URL="
                    f"{shlex.quote(mock_base_url)}; capsem-doctor"
                ),
                "timeout_secs": 220,
            },
            timeout=240,
        )
        assert exec_resp is not None, "doctor exec returned no body"
        stdout = exec_resp.get("stdout", "")
        stderr = exec_resp.get("stderr", "")
        output = stdout + stderr
        assert exec_resp.get("exit_code") == 0, exec_resp
        assert "failed" not in output.lower()
        assert "capsem_test_oauth_access_0123456789abcdef" not in output
        assert "capsem_test_openai_api_key" not in output

        history = client.get(f"/vms/{session_id}/history", timeout=30)
        assert history is not None
        assert history.get("total", 0) >= 2
        history_commands = [entry.get("command") or "" for entry in history.get("commands", [])]
        assert any("capsem-doctor" in command for command in history_commands)

        counts = client.get(f"/vms/{session_id}/history/counts", timeout=30)
        assert counts is not None
        assert counts["exec_count"] >= 2
        assert counts["audit_count"] >= 0

        security_latest = client.get(f"/vms/{session_id}/security/latest?limit=25", timeout=30)
        assert isinstance(security_latest, list)
        assert len(security_latest) > 0
        assert all(set(row) == EXPECTED_SECURITY_LATEST_FIELDS for row in security_latest)
        assert all(row["event_id"] for row in security_latest)
        assert all(row["rule_id"] for row in security_latest)
        assert all(row["rule_action"] in {"allow", "ask", "block", "preprocess", "rewrite", "postprocess"} for row in security_latest)
        assert all(row["detection_level"] in {"none", "informational", "low", "medium", "high", "critical"} for row in security_latest)
        assert all(json.loads(row["rule_json"]) for row in security_latest)
        assert all(json.loads(row["event_json"]) for row in security_latest)

        conn = _connect_session_db(service.tmp_dir / "sessions", session_id)
        for table in (
            "net_events",
            "dns_events",
            "mcp_calls",
            "model_calls",
            "tool_calls",
            "fs_events",
            "exec_events",
            "security_rule_events",
            "substitution_events",
        ):
            assert _count(conn, table) > 0, f"{table} should contain doctor evidence"
            assert "event_id" in _table_columns(conn, table), f"{table} must carry event_id"
        assert _table_columns(conn, "substitution_events") == EXPECTED_SUBSTITUTION_COLUMNS

        model_net = _single(
            conn,
            """
            SELECT *
            FROM net_events
            WHERE path = '/v1/chat/completions'
            ORDER BY id DESC
            LIMIT 1
            """,
        )
        _assert_ledger_id(model_net["event_id"])
        assert model_net["method"] == "POST"
        assert model_net["status_code"] == 200
        assert model_net["decision"] == "allowed"
        assert model_net["bytes_sent"] > 0
        assert model_net["bytes_received"] > 0
        assert model_net["credential_ref"].startswith("credential:blake3:")
        assert "capsem_test_openai_api_key" not in (model_net["request_headers"] or "")
        assert "capsem_test_openai_api_key" not in (model_net["request_body_preview"] or "")

        model_call = _single(
            conn,
            """
            SELECT *
            FROM model_calls
            WHERE trace_id = ?
              AND path = '/v1/chat/completions'
            ORDER BY id DESC
            LIMIT 1
            """,
            (model_net["trace_id"],),
        )
        _assert_ledger_id(model_call["event_id"])
        assert model_call["event_id"] != model_net["event_id"]
        assert model_call["trace_id"] == model_net["trace_id"]
        assert model_call["provider"] == "openai"
        assert model_call["model"] == "mock-local"
        assert model_call["method"] == "POST"
        assert model_call["path"] == "/v1/chat/completions"
        assert model_call["input_tokens"] > 0
        assert model_call["output_tokens"] > 0
        assert model_call["credential_ref"] == model_net["credential_ref"]

        http_security = _single(
            conn,
            """
            SELECT *
            FROM security_rule_events
            WHERE event_id = ?
              AND event_type = 'http.request'
            ORDER BY id DESC
            LIMIT 1
            """,
            (model_net["event_id"],),
        )
        assert http_security["rule_action"] == "allow"
        assert http_security["rule_id"]

        model_security = _single(
            conn,
            """
            SELECT *
            FROM security_rule_events
            WHERE event_id = ?
              AND event_type = 'model.call'
            ORDER BY id DESC
            LIMIT 1
            """,
            (model_call["event_id"],),
        )
        assert model_security["rule_action"] == "allow"
        assert model_security["detection_level"] in {"none", "informational"}
        assert model_security["rule_id"]
        assert model_security["event_json"]
        assert model_security["rule_json"]

        tool_call = _single(
            conn,
            "SELECT * FROM tool_calls WHERE tool_name = 'fixture_lookup' ORDER BY id DESC LIMIT 1",
        )
        _assert_ledger_id(tool_call["event_id"])
        assert tool_call["provider"] == "openai"
        assert tool_call["origin"] == "native"
        assert tool_call["status"] in {"requested", "observed"}
        assert tool_call["credential_ref"] == model_call["credential_ref"]
        assert tool_call["trace_id"] == model_call["trace_id"]

        mcp_methods = {
            row["method"]
            for row in conn.execute("SELECT DISTINCT method FROM mcp_calls").fetchall()
        }
        assert {"initialize", "tools/list", "tools/call"}.issubset(mcp_methods)
        mcp_call = _single(
            conn,
            "SELECT * FROM mcp_calls WHERE method = 'tools/call' ORDER BY id DESC LIMIT 1",
        )
        _assert_ledger_id(mcp_call["event_id"])
        assert mcp_call["decision"] in {"allowed", "denied", "ask", "error"}
        assert mcp_call["server_name"]
        assert mcp_call["tool_name"]

        broker_outcomes = {
            row["outcome"]
            for row in conn.execute("SELECT DISTINCT outcome FROM substitution_events").fetchall()
        }
        assert broker_outcomes
        assert broker_outcomes <= BROKER_OUTCOMES
        assert broker_outcomes <= HAPPY_PATH_BROKER_OUTCOMES
        credential_sources = {
            row["source"]
            for row in conn.execute(
                "SELECT DISTINCT source FROM substitution_events WHERE outcome = 'captured'"
            ).fetchall()
        }
        assert "http.header.authorization" in credential_sources
        assert "http.body.response.$.access_token" in credential_sources
        assert "http.body.response.$.refresh_token" in credential_sources
        credential_refs = [
            row["substitution_ref"]
            for row in conn.execute(
                "SELECT substitution_ref FROM substitution_events WHERE outcome = 'captured'"
            ).fetchall()
        ]
        assert credential_refs
        assert all(ref.startswith("credential:blake3:") for ref in credential_refs)
        assert all(len(ref.removeprefix("credential:blake3:")) == 64 for ref in credential_refs)
        substitution_rows = conn.execute("SELECT * FROM substitution_events").fetchall()
        assert all(row["material_class"] == "credential" for row in substitution_rows)
        assert all(row["algorithm"] == "blake3" for row in substitution_rows)
        assert all(
            row["event_type"] in {"http.request", "http.response", "model.call"}
            for row in substitution_rows
        )
        assert all(
            row["confidence"] is None or 0.0 <= float(row["confidence"]) <= 1.0
            for row in substitution_rows
        )
        assert all(
            json.loads(row["context_json"]) if row["context_json"] else True
            for row in substitution_rows
        )

        dns = _single(conn, "SELECT * FROM dns_events ORDER BY id DESC LIMIT 1")
        _assert_ledger_id(dns["event_id"])
        assert dns["qname"]
        assert dns["source_proto"] in {"udp", "tcp"}
        assert dns["decision"] in {"allowed", "denied"}

        fs = _single(conn, "SELECT * FROM fs_events ORDER BY id DESC LIMIT 1")
        _assert_ledger_id(fs["event_id"])
        assert fs["action"] in {"created", "modified", "deleted", "restored"}
        assert fs["path"]

        exec_row = _single(
            conn,
            "SELECT * FROM exec_events WHERE command LIKE '%capsem-doctor%' ORDER BY id DESC LIMIT 1",
        )
        _assert_ledger_id(exec_row["event_id"])
        assert exec_row["exit_code"] == 0
        assert exec_row["source"] in {"api", "cli", "mcp"}
        assert exec_row["stdout_bytes"] > 0
        _assert_no_raw_secret_markers_in_session_db(conn)
        conn.close()
    finally:
        stop_process(mock_proc)
        if client is not None:
            try:
                client.delete(f"/vms/{session_id}/delete", timeout=60)
            except Exception:
                pass
        service.stop()
