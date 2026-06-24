"""Ironbank black-box capsem-doctor ledger tests."""

from __future__ import annotations

import json
import re
import shlex
import sqlite3
import subprocess
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

EXPECTED_MCP_SERVER_FIELDS = {
    "name",
    "url",
    "has_auth_credential",
    "custom_header_count",
    "source",
    "enabled",
    "running",
    "tool_count",
    "is_stdio",
}

EXPECTED_MCP_TOOL_FIELDS = {
    "namespaced_name",
    "original_name",
    "description",
    "server_name",
    "annotations",
    "pin_hash",
    "pin_changed",
    "permission_action",
    "permission_source",
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
EICAR_TEXT = "X5O!P%@AP[4\\PZX54(P^)7CC)7}$EICAR-STANDARD-ANTIVIRUS-TEST-FILE!$H+H*"


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


def _post_bytes_with_status(
    socket_path: Path, path: str, data: bytes, timeout: int = 60
) -> tuple[int, bytes]:
    result = subprocess.run(
        [
            "curl",
            "-s",
            "-S",
            "-o",
            "-",
            "-w",
            "\n__STATUS__%{http_code}",
            "--unix-socket",
            str(socket_path),
            "-X",
            "POST",
            "-H",
            "Content-Type: application/octet-stream",
            "--max-time",
            str(timeout),
            "--data-binary",
            "@-",
            f"http://localhost{path}",
        ],
        input=data,
        capture_output=True,
        timeout=timeout + 5,
    )
    if result.returncode != 0:
        raise ConnectionError(f"curl failed: {result.stderr.decode(errors='replace')}")
    sep = b"\n__STATUS__"
    idx = result.stdout.rfind(sep)
    assert idx != -1, result.stdout
    return int(result.stdout[idx + len(sep) :].decode(errors="replace")), result.stdout[:idx]


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
        assert exec_resp.get("exit_code") == 0, (
            f"capsem-doctor failed with exit {exec_resp.get('exit_code')}\n"
            f"STDOUT:\n{stdout}\n"
            f"STDERR:\n{stderr}\n"
            f"response keys={sorted(exec_resp)}"
        )
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

        mcp_default = client.get(f"/profiles/{CODE_PROFILE_ID}/mcp/default/info", timeout=30)
        assert set(mcp_default) == {"action", "source", "rule_id"}
        assert mcp_default["action"] in {"allow", "ask", "block", "disable"}
        assert mcp_default["source"]

        mcp_servers = client.get(f"/profiles/{CODE_PROFILE_ID}/mcp/servers/list", timeout=30)
        assert isinstance(mcp_servers, list)
        assert mcp_servers
        assert all(set(server) == EXPECTED_MCP_SERVER_FIELDS for server in mcp_servers)
        local_server = next(server for server in mcp_servers if server["name"] == "local")
        assert local_server["enabled"] is True
        assert local_server["is_stdio"] is True
        assert local_server["tool_count"] >= 3
        assert local_server["url"] == ""

        mcp_tools = client.get(
            f"/profiles/{CODE_PROFILE_ID}/mcp/servers/local/tools/list",
            timeout=30,
        )
        assert isinstance(mcp_tools, list)
        assert mcp_tools
        assert all(set(tool) == EXPECTED_MCP_TOOL_FIELDS for tool in mcp_tools)
        tools_by_name = {tool["original_name"]: tool for tool in mcp_tools}
        for tool_name in ("fetch_http", "grep_http", "http_headers"):
            tool = tools_by_name[tool_name]
            assert tool["server_name"] == "local"
            assert tool["namespaced_name"] == f"local__{tool_name}"
            assert tool["description"]
            assert tool["pin_changed"] is False
            assert tool["permission_action"] in {"allow", "ask", "block", "disable"}
            assert tool["permission_source"]

        conn = _connect_session_db(service.tmp_dir / "sessions", session_id)
        assert "mcp_calls" not in {
            row["name"]
            for row in conn.execute(
                "SELECT name FROM sqlite_master WHERE type = 'table'"
            ).fetchall()
        }
        for table in (
            "net_events",
            "dns_events",
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
        assert model_call["provider"] == "unknown"
        assert model_call["protocol"] == "openai"
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

        security_rows = conn.execute("SELECT * FROM security_rule_events").fetchall()
        security_actions = {row["rule_action"] for row in security_rows}
        security_levels = {row["detection_level"] for row in security_rows}
        assert {"allow", "ask"} <= security_actions
        assert {"none", "informational"} <= security_levels
        assert security_actions <= {"allow", "ask", "block", "preprocess", "rewrite", "postprocess"}
        assert security_levels <= {"none", "informational", "low", "medium", "high", "critical"}

        ask_rows = [row for row in security_rows if row["rule_action"] == "ask"]
        assert ask_rows, "doctor must trigger the default local-network ask guard"
        for row in ask_rows:
            payload = json.loads(row["event_json"])
            assert row["event_type"] == "http.request"
            assert row["rule_id"] == "profiles.rules.default_000_local_network"
            assert row["detection_level"] == "none"
            assert payload["decision"]["effective"] == "allow"
            sibling_actions = {
                sibling["rule_action"]
                for sibling in security_rows
                if sibling["event_id"] == row["event_id"]
            }
            sibling_rules = {
                sibling["rule_id"]
                for sibling in security_rows
                if sibling["event_id"] == row["event_id"]
            }
            assert "allow" in sibling_actions
            assert "profiles.rules.capsem_mock_server" in sibling_rules

        informational_rows = [
            row for row in security_rows if row["detection_level"] == "informational"
        ]
        assert informational_rows, "doctor must emit informational detection rows"
        for row in informational_rows:
            payload = json.loads(row["event_json"])
            detections = payload.get("detections", [])
            assert any(
                detection.get("detection_level") == "informational"
                and detection.get("rule_id") == row["rule_id"]
                for detection in detections
            )

        plugin_executions = [
            execution
            for row in security_rows
            for execution in json.loads(row["event_json"]).get("plugin_executions", [])
        ]
        assert plugin_executions, "doctor security payloads must carry plugin timings"
        assert {
            "plugin_id",
            "stage",
            "applied",
            "duration_us",
        } <= set(plugin_executions[0])
        assert all(
            execution["stage"] in {"preprocess", "postprocess", "logging"}
            for execution in plugin_executions
        )
        assert all(isinstance(execution["applied"], bool) for execution in plugin_executions)
        assert all(isinstance(execution["duration_us"], int) for execution in plugin_executions)
        assert any(execution["plugin_id"] == "credential_broker" for execution in plugin_executions)
        assert any(
            execution["plugin_id"] == "log_sanitizer" and execution["applied"] is True
            for execution in plugin_executions
        )

        tool_call = _single(
            conn,
            "SELECT * FROM tool_calls WHERE tool_name = 'fixture_lookup' ORDER BY id DESC LIMIT 1",
        )
        _assert_ledger_id(tool_call["event_id"])
        assert tool_call["provider"] == "unknown"
        assert tool_call["origin"] == "native"
        assert tool_call["status"] in {"requested", "observed"}
        assert tool_call["credential_ref"] == model_call["credential_ref"]
        assert tool_call["trace_id"] == model_call["trace_id"]

        mcp_call = _single(
            conn,
            "SELECT * FROM tool_calls WHERE origin = 'mcp' AND method = 'tools/call' ORDER BY id DESC LIMIT 1",
        )
        _assert_ledger_id(mcp_call["event_id"])
        assert mcp_call["decision"] in {"allowed", "denied", "ask", "error"}
        assert mcp_call["server_name"]
        assert mcp_call["tool_name"]
        assert mcp_call["process_name"] != "MainThread"
        assert mcp_call["bytes_sent"] > 0
        assert mcp_call["arguments"]

        mcp_fetch = _single(
            conn,
            """
            SELECT *
            FROM tool_calls
            WHERE method = 'tools/call'
              AND tool_name LIKE '%fetch_http%'
              AND origin = 'mcp'
            ORDER BY id DESC
            LIMIT 1
            """,
        )
        _assert_ledger_id(mcp_fetch["event_id"])
        assert mcp_fetch["server_name"] == "local"
        assert mcp_fetch["tool_name"] in {"fetch_http", "local__fetch_http"}
        assert mcp_fetch["decision"] == "allowed"
        assert mcp_fetch["bytes_sent"] > 0
        assert mcp_fetch["bytes_received"] > 0
        assert "fetch_http" in mcp_fetch["arguments"]
        assert "Capsem local pagination fixture" in (mcp_fetch["response_preview"] or "")

        mcp_net = _single(
            conn,
            """
            SELECT *
            FROM net_events
            WHERE conn_type = 'mcp_builtin'
            ORDER BY id DESC
            LIMIT 1
            """,
        )
        _assert_ledger_id(mcp_net["event_id"])
        assert mcp_net["decision"] == "allowed"
        assert mcp_net["bytes_sent"] >= 0
        assert mcp_net["bytes_received"] > 0

        mcp_security = _single(
            conn,
            """
            SELECT *
            FROM security_rule_events
            WHERE event_id = ?
            ORDER BY id DESC
            LIMIT 1
            """,
            (mcp_fetch["event_id"],),
        )
        assert mcp_security["event_type"] == "mcp.tool_call"
        assert mcp_security["rule_action"] in {"allow", "ask"}
        assert mcp_security["rule_id"]
        assert json.loads(mcp_security["event_json"])
        assert json.loads(mcp_security["rule_json"])

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
        assert all(row["confidence"] is None for row in substitution_rows)
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


def test_runtime_plugin_action_matrix_pays_file_import_ledger_debt():
    assert ASSETS_DIR.exists(), f"{ASSETS_DIR} missing; build VM assets before Ironbank"
    assert PROFILES_DIR.exists(), f"{PROFILES_DIR} missing; materialize profile config before Ironbank"

    service = ServiceInstance()
    client = None
    session_id = vm_name("ironbank-plugin")
    try:
        service.start()
        client = service.client()

        enabled_pre = client.patch(
            f"/profiles/{CODE_PROFILE_ID}/plugins/dummy_pre_eicar/edit",
            {"mode": "block", "detection_level": "critical"},
            timeout=30,
        )
        assert enabled_pre["id"] == "dummy_pre_eicar"
        assert enabled_pre["config"]["mode"] == "block"
        assert enabled_pre["config"]["detection_level"] == "critical"
        assert enabled_pre["runtime"]["enabled"] is True

        enabled_post = client.patch(
            f"/profiles/{CODE_PROFILE_ID}/plugins/dummy_post_allow/edit",
            {"mode": "allow", "detection_level": "low"},
            timeout=30,
        )
        assert enabled_post["id"] == "dummy_post_allow"
        assert enabled_post["config"]["mode"] == "allow"
        assert enabled_post["config"]["detection_level"] == "low"
        assert enabled_post["runtime"]["enabled"] is True

        create = client.post(
            "/vms/create",
            {
                "name": session_id,
                "profile_id": CODE_PROFILE_ID,
                "ram_mb": DEFAULT_RAM_MB,
                "cpus": DEFAULT_CPUS,
            },
            timeout=90,
        )
        assert create is not None
        assert create.get("id") == session_id or create.get("name") == session_id
        assert wait_exec_ready(client, session_id, timeout=EXEC_READY_TIMEOUT)

        blocked_status, blocked_body = _post_bytes_with_status(
            service.uds_path,
            f"/vms/{session_id}/files/content?path=eicar-blocked.txt",
            EICAR_TEXT.encode(),
            timeout=30,
        )
        assert blocked_status in {400, 403, 409, 500}, blocked_body
        assert b"EICAR" not in blocked_body

        get_status, _ = client.get_bytes(
            f"/vms/{session_id}/files/content?path=eicar-blocked.txt",
            timeout=30,
        )
        assert get_status in {404, 500}

        rewrite_pre = client.patch(
            f"/profiles/{CODE_PROFILE_ID}/plugins/dummy_pre_eicar/edit",
            {"mode": "rewrite", "detection_level": "medium"},
            timeout=30,
        )
        assert rewrite_pre["id"] == "dummy_pre_eicar"
        assert rewrite_pre["config"]["mode"] == "rewrite"
        assert rewrite_pre["config"]["detection_level"] == "medium"
        assert rewrite_pre["runtime"]["enabled"] is True

        rewrite_status, rewrite_body = _post_bytes_with_status(
            service.uds_path,
            f"/vms/{session_id}/files/content?path=eicar-rewrite.txt",
            EICAR_TEXT.encode(),
            timeout=30,
        )
        assert rewrite_status == 200, rewrite_body
        rewrite_json = json.loads(rewrite_body)
        assert rewrite_json["success"] is True
        assert rewrite_json["size"] != len(EICAR_TEXT.encode())

        rewrite_read_status, rewrite_read_body = client.get_bytes(
            f"/vms/{session_id}/files/content?path=eicar-rewrite.txt",
            timeout=30,
        )
        assert rewrite_read_status == 200
        rewrite_content = rewrite_read_body.decode()
        assert rewrite_content == "[capsem-rewritten-eicar]"
        assert "EICAR-STANDARD-ANTIVIRUS-TEST-FILE" not in rewrite_content

        disabled_pre = client.patch(
            f"/profiles/{CODE_PROFILE_ID}/plugins/dummy_pre_eicar/edit",
            {"mode": "disable", "detection_level": "informational"},
            timeout=30,
        )
        assert disabled_pre["id"] == "dummy_pre_eicar"
        assert disabled_pre["config"]["mode"] == "disable"
        assert disabled_pre["runtime"]["enabled"] is False

        allowed_status, allowed_body = _post_bytes_with_status(
            service.uds_path,
            f"/vms/{session_id}/files/content?path=eicar-allowed.txt",
            EICAR_TEXT.encode(),
            timeout=30,
        )
        assert allowed_status == 200, allowed_body
        allowed_json = json.loads(allowed_body)
        assert allowed_json["success"] is True

        read_status, read_body = client.get_bytes(
            f"/vms/{session_id}/files/content?path=eicar-allowed.txt",
            timeout=30,
        )
        assert read_status == 200
        assert read_body.decode() == EICAR_TEXT

        conn = _connect_session_db(service.tmp_dir / "sessions", session_id)
        security_rows = conn.execute(
            """
            SELECT *
            FROM security_rule_events
            WHERE event_type = 'file.import'
            ORDER BY id
            """
        ).fetchall()
        assert security_rows, "file imports must emit security ledger rows"
        assert {row["rule_action"] for row in security_rows} == {"allow"}
        payloads = [json.loads(row["event_json"]) for row in security_rows]
        assert {"block", "allow"} <= {
            payload["decision"]["effective"] for payload in payloads
        }

        blocked_rows = [
            row
            for row in security_rows
            if json.loads(row["event_json"])["decision"]["effective"] == "block"
        ]
        assert blocked_rows, "enabled dummy_pre_eicar must produce block evidence"
        blocked_payloads = [json.loads(row["event_json"]) for row in blocked_rows]
        assert any(payload["decision"]["effective"] == "block" for payload in blocked_payloads)
        assert any(
            detection.get("source") == "plugin"
            and detection.get("plugin_id") == "dummy_pre_eicar"
            and detection.get("plugin_mode") == "block"
            and detection.get("detection_level") == "critical"
            for payload in blocked_payloads
            for detection in payload.get("detections", [])
        )
        assert any(
            detection.get("source") == "plugin"
            and detection.get("plugin_id") == "dummy_post_allow"
            and detection.get("plugin_mode") == "allow"
            and detection.get("detection_level") == "low"
            for payload in blocked_payloads
            for detection in payload.get("detections", [])
        )

        plugin_executions = [
            execution
            for payload in blocked_payloads
            for execution in payload.get("plugin_executions", [])
        ]
        assert any(
            execution["plugin_id"] == "dummy_pre_eicar"
            and execution["stage"] == "preprocess"
            and execution["applied"] is True
            for execution in plugin_executions
        )
        assert any(
            execution["plugin_id"] == "dummy_post_allow"
            and execution["stage"] == "postprocess"
            and execution["applied"] is True
            for execution in plugin_executions
        )
        assert all(payload["decision"]["effective"] == "block" for payload in blocked_payloads)

        rewrite_file_row = _single(
            conn,
            """
            SELECT *
            FROM fs_events
            WHERE path = 'eicar-rewrite.txt'
              AND action = 'import'
            ORDER BY id DESC
            LIMIT 1
            """,
        )
        _assert_ledger_id(rewrite_file_row["event_id"])
        rewrite_security = [
            row for row in security_rows if row["event_id"] == rewrite_file_row["event_id"]
        ]
        assert rewrite_security, "rewrite-mode import must carry security rows"
        rewrite_payloads = [json.loads(row["event_json"]) for row in rewrite_security]
        assert all(payload["decision"]["effective"] == "allow" for payload in rewrite_payloads)
        assert any(
            detection.get("source") == "plugin"
            and detection.get("plugin_id") == "dummy_pre_eicar"
            and detection.get("plugin_mode") == "rewrite"
            and detection.get("detection_level") == "medium"
            for payload in rewrite_payloads
            for detection in payload.get("detections", [])
        )
        assert any(
            execution["plugin_id"] == "dummy_pre_eicar"
            and execution["stage"] == "preprocess"
            and execution["applied"] is True
            for payload in rewrite_payloads
            for execution in payload.get("plugin_executions", [])
        )

        allowed_file_row = _single(
            conn,
            """
            SELECT *
            FROM fs_events
            WHERE path = 'eicar-allowed.txt'
              AND action = 'import'
            ORDER BY id DESC
            LIMIT 1
            """,
        )
        _assert_ledger_id(allowed_file_row["event_id"])
        assert allowed_file_row["size"] == len(EICAR_TEXT.encode())
        allowed_security = [
            row for row in security_rows if row["event_id"] == allowed_file_row["event_id"]
        ]
        assert allowed_security, "successful import must carry security rows"
        assert {row["rule_action"] for row in allowed_security} == {"allow"}
        assert all(
            json.loads(row["event_json"])["decision"]["effective"] == "allow"
            for row in allowed_security
        )

        plugins = client.get(f"/profiles/{CODE_PROFILE_ID}/plugins/list", timeout=30)
        by_id = {plugin["id"]: plugin for plugin in plugins["plugins"]}
        assert by_id["dummy_pre_eicar"]["runtime"]["enabled"] is False
        assert by_id["dummy_post_allow"]["runtime"]["enabled"] is True
        assert by_id["dummy_post_allow"]["runtime"]["execution_count"] == 0
        dummy_post_detail = client.get(
            f"/profiles/{CODE_PROFILE_ID}/plugins/dummy_post_allow/info",
            timeout=30,
        )
        assert dummy_post_detail["runtime"]["enabled"] is True
        assert dummy_post_detail["runtime"]["execution_count"] >= 1
        conn.close()
    finally:
        if client is not None:
            try:
                client.delete(f"/vms/{session_id}/delete", timeout=60)
            except Exception:
                pass
        service.stop()
