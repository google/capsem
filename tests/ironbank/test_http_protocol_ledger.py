"""Ironbank HTTP protocol ledger contract tests."""

from __future__ import annotations

from contextlib import closing
import json
import os
from pathlib import Path
import re
import sqlite3
import textwrap
import uuid

import pytest

from helpers.constants import CODE_PROFILE_ID, DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.gateway import GatewayInstance, TcpHttpClient
from helpers.mock_server import MOCK_SERVER_BINARY, start_mock_server, stop_process
from helpers.service import ServiceInstance, wait_exec_ready, vm_name

pytestmark = pytest.mark.integration

PROJECT_ROOT = Path(__file__).resolve().parents[2]
ASSETS_DIR = PROJECT_ROOT / "assets"
PROFILES_DIR = PROJECT_ROOT / "target" / "config" / "profiles"

EXPECTED_NET_COLUMNS = {
    "id",
    "event_id",
    "timestamp",
    "domain",
    "port",
    "decision",
    "process_name",
    "pid",
    "method",
    "path",
    "query",
    "status_code",
    "bytes_sent",
    "bytes_received",
    "duration_ms",
    "matched_rule",
    "request_headers",
    "response_headers",
    "request_body_preview",
    "response_body_preview",
    "conn_type",
    "policy_mode",
    "policy_action",
    "policy_rule",
    "policy_reason",
    "trace_id",
    "credential_ref",
}

EXPECTED_SECURITY_COLUMNS = {
    "id",
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


def _connect_session_db(service: ServiceInstance, session_id: str) -> sqlite3.Connection:
    db_path = service.tmp_dir / "sessions" / session_id / "session.db"
    assert db_path.exists(), f"session DB missing at {db_path}"
    conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    conn.row_factory = sqlite3.Row
    return conn


def _table_columns(conn: sqlite3.Connection, table: str) -> set[str]:
    return {row[1] for row in conn.execute(f"PRAGMA table_info({table})").fetchall()}


def _query_rows(client, session_id: str, sql: str) -> list[dict]:
    payload = client.post(f"/vms/{session_id}/inspect", {"sql": sql}, timeout=30)
    assert set(payload) == {"columns", "rows"}
    return [dict(zip(payload["columns"], row, strict=True)) for row in payload["rows"]]


def _event_id(value: object) -> str:
    assert isinstance(value, str)
    assert len(value) == 12
    assert all(ch in "0123456789abcdef" for ch in value)
    return value


def _one_json_line(stdout: str, prefix: str) -> dict:
    line = next((line for line in stdout.splitlines() if line.startswith(prefix)), None)
    assert line is not None, stdout
    return json.loads(line.split("=", 1)[1])


def test_plain_json_http_request_pays_full_ledger_debt_blackbox() -> None:
    assert MOCK_SERVER_BINARY.exists(), f"{MOCK_SERVER_BINARY} missing"
    assert ASSETS_DIR.exists(), f"{ASSETS_DIR} missing; build VM assets before Ironbank"
    assert PROFILES_DIR.exists(), f"{PROFILES_DIR} missing; materialize profile config"

    service = ServiceInstance()
    gateway: GatewayInstance | None = None
    mock_proc = None
    client = None
    session_id = vm_name("ironbank-http")
    nonce = uuid.uuid4().hex
    old_corp_config = os.environ.get("CAPSEM_CORP_CONFIG")
    try:
        mock_proc, ready = start_mock_server(
            request_log=service.tmp_dir / "upstream-http-transcript.jsonl"
        )
        corp_path = service.tmp_dir / "corp.toml"
        corp_path.write_text(
            textwrap.dedent(
                """
                refresh_policy = "24h"

                [settings."vm.resources.log_bodies"]
                value = true
                modified = "2026-06-14T00:00:00Z"

                [settings."vm.resources.max_body_capture"]
                value = 8192
                modified = "2026-06-14T00:00:00Z"

                [settings."security.web.http_upstream_ports"]
                value = [80, 3713, 8080]
                modified = "2026-06-14T00:00:00Z"

                [corp.rules.allow_ironbank_mock_http]
                name = "allow_ironbank_mock_http"
                action = "allow"
                priority = -100
                reason = "Allow the hermetic Ironbank HTTP fixture while keeping local-network ask defaults intact."
                match = 'http.host == "127.0.0.1" && tcp.port == "3713" && http.path == "/echo"'
                """
            ).strip()
            + "\n",
            encoding="utf-8",
        )
        os.environ["CAPSEM_CORP_CONFIG"] = str(corp_path)
        service.start()
        client = service.client()
        gateway = GatewayInstance(uds_path=service.uds_path)
        gateway.start()
        gateway_client = TcpHttpClient(gateway.base_url, gateway.token)

        create = client.post(
            "/vms/create",
            {
                "name": session_id,
                "profile_id": CODE_PROFILE_ID,
                "ram_mb": DEFAULT_RAM_MB,
                "cpus": DEFAULT_CPUS,
                "env": {"CAPSEM_MOCK_SERVER_BASE_URL": ready["base_url"]},
            },
            timeout=90,
        )
        assert create is not None
        assert create.get("id") == session_id or create.get("name") == session_id
        assert wait_exec_ready(client, session_id, timeout=EXEC_READY_TIMEOUT)

        script = textwrap.dedent(
            f"""
            import json
            import urllib.request

            payload = {{"kind": "ironbank_http_plain_json", "nonce": {json.dumps(nonce)}}}
            body = json.dumps(payload, sort_keys=True, separators=(",", ":")).encode()
            req = urllib.request.Request(
                {json.dumps(ready["base_url"].rstrip("/") + "/echo?case=plain-json")},
                data=body,
                method="POST",
                headers={{
                    "content-type": "application/json",
                    "user-agent": "capsem-ironbank-http/1",
                    "x-ironbank-nonce": {json.dumps(nonce)},
                }},
            )
            with urllib.request.urlopen(req, timeout=30) as response:
                response_body = response.read().decode()
                result = {{
                    "status": response.status,
                    "content_type": response.headers.get("content-type"),
                    "body": json.loads(response_body),
                    "request_body": body.decode(),
                    "nonce": {json.dumps(nonce)},
                }}
            print("IRONBANK_HTTP_RESULT=" + json.dumps(result, sort_keys=True))
            """
        ).strip()
        upload = client.post_bytes(
            f"/vms/{session_id}/files/content?path=ironbank-http.py",
            script.encode(),
            timeout=30,
        )
        assert upload is not None
        assert upload["success"] is True

        exec_resp = client.post(
            f"/vms/{session_id}/exec",
            {"command": "python3 /root/ironbank-http.py", "timeout_secs": 120},
            timeout=150,
        )
        assert exec_resp is not None
        assert exec_resp["exit_code"] == 0, exec_resp
        result = _one_json_line(exec_resp.get("stdout") or "", "IRONBANK_HTTP_RESULT=")
        assert result["status"] == 200
        assert result["content_type"].startswith("application/json")
        assert result["body"]["method"] == "POST"
        assert result["body"]["path"] == "/echo"
        assert result["body"]["content_type"] == "application/json"
        assert result["body"]["user_agent"] == "capsem-ironbank-http/1"
        assert result["body"]["body_size"] == len(result["request_body"])
        assert result["body"]["has_authorization"] is False
        assert result["body"]["authorization_is_broker_ref"] is False
        assert result["nonce"] == nonce
        assert nonce in result["request_body"]

        request_log_path = Path(ready["request_log"])
        upstream_text = (
            request_log_path.read_text(encoding="utf-8") if request_log_path.exists() else ""
        )
        upstream_records = [
            json.loads(line) for line in upstream_text.splitlines() if line.strip()
        ]
        upstream_echo = [row for row in upstream_records if row["path"] == "/echo"]
        assert len(upstream_echo) == 1, upstream_records
        assert upstream_echo[0]["method"] == "POST"
        assert upstream_echo[0]["query"] == "case=plain-json"
        assert upstream_echo[0]["status"] == 200
        assert upstream_echo[0]["request_body"] == result["request_body"]
        assert upstream_echo[0]["headers"]["x-ironbank-nonce"] == nonce

        with closing(_connect_session_db(service, session_id)) as conn:
            assert _table_columns(conn, "net_events") == EXPECTED_NET_COLUMNS
            assert _table_columns(conn, "security_rule_events") == EXPECTED_SECURITY_COLUMNS
            rows = conn.execute(
                """
                SELECT * FROM net_events
                WHERE method = 'POST' AND path = '/echo' AND query = 'case=plain-json'
                ORDER BY id
                """
            ).fetchall()
            assert len(rows) == 1, [dict(row) for row in rows]
            net = dict(rows[0])
            event_id = _event_id(net["event_id"])
            assert net["domain"] == "127.0.0.1"
            assert net["port"] == 3713
            assert net["decision"] == "allowed"
            assert net["status_code"] == 200
            assert net["bytes_sent"] >= len(result["request_body"])
            assert net["bytes_received"] == upstream_echo[0]["response_bytes"]
            assert isinstance(net["duration_ms"], int)
            assert net["duration_ms"] >= 0
            assert net["matched_rule"] == "corp.rules.allow_ironbank_mock_http"
            assert net["policy_action"] == "allow"
            assert net["policy_rule"] == "corp.rules.allow_ironbank_mock_http"
            assert net["credential_ref"] is None
            assert net["conn_type"] == "http-mitm"
            assert nonce not in net["request_headers"]
            assert re.search(
                r"x-ironbank-nonce: hash:[0-9a-f]{12}",
                net["request_headers"].lower(),
            )
            assert "content-type: application/json" in net["request_headers"].lower()
            assert "content-type: application/json" in net["response_headers"].lower()
            assert net["request_body_preview"] == result["request_body"]
            response_preview = json.loads(net["response_body_preview"])
            assert response_preview["path"] == "/echo"
            assert response_preview["body_size"] == len(result["request_body"])
            assert response_preview["has_authorization"] is False
            assert isinstance(net["trace_id"], str) and net["trace_id"]

            security_rows = conn.execute(
                """
                SELECT * FROM security_rule_events
                WHERE event_id = ? AND event_type = 'http.request'
                ORDER BY id
                """,
                (event_id,),
            ).fetchall()
            assert len(security_rows) >= 1, [dict(row) for row in security_rows]
            default_rule = next(
                row
                for row in security_rows
                if row["rule_id"] == "corp.rules.allow_ironbank_mock_http"
            )
            assert default_rule["rule_action"] == "allow"
            assert default_rule["detection_level"] == "none"
            assert default_rule["trace_id"] == net["trace_id"]
            event_json = json.loads(default_rule["event_json"])
            assert event_json["event_type"] == "http.request"
            assert event_json["http"]["host"] == "127.0.0.1"
            assert event_json["http"]["method"] == "POST"
            assert event_json["http"]["path"] == "/echo"
            assert event_json["http"]["query"] == "case=plain-json"
            assert event_json["http"]["status"] == "200"
            assert event_json["http"]["body"].find(nonce) != -1
            assert event_json["tcp"]["port"] == "3713"
            assert event_json["ip"]["value"] == "127.0.0.1"
            assert event_json["ip"]["version"] == "4"

        uds_rows = _query_rows(
            client,
            session_id,
            """
            SELECT event_id, domain, port, method, path, query, status_code, decision,
                   bytes_sent, bytes_received, matched_rule, request_body_preview,
                   response_body_preview, conn_type, trace_id
            FROM net_events
            WHERE event_id = '%s'
            """
            % event_id,
        )
        assert len(uds_rows) == 1
        assert uds_rows[0]["event_id"] == event_id
        assert uds_rows[0]["request_body_preview"] == result["request_body"]
        assert json.loads(uds_rows[0]["response_body_preview"])["path"] == "/echo"

        status, gateway_body = gateway_client.get_status_and_body(
            f"/vms/{session_id}/inspect",
            timeout=30,
            extra_headers={"content-type": "application/json"},
        )
        assert status == 405 or status == 400
        gateway_rows = gateway_client.post(
            f"/vms/{session_id}/inspect",
            {
                "sql": (
                    "SELECT event_id, method, path, status_code, decision, trace_id "
                    f"FROM net_events WHERE event_id = '{event_id}'"
                )
            },
            timeout=30,
        )
        assert set(gateway_rows) == {"columns", "rows"}
        assert gateway_rows["columns"] == [
            "event_id",
            "method",
            "path",
            "status_code",
            "decision",
            "trace_id",
        ]
        assert gateway_rows["rows"] == [[event_id, "POST", "/echo", 200, "allowed", net["trace_id"]]]

        timeline = client.get(
            f"/vms/{session_id}/timeline?trace_id={net['trace_id']}&layers=net&limit=10",
            timeout=30,
        )
        assert set(timeline) == {"columns", "rows"}
        timeline_rows = [dict(zip(timeline["columns"], row, strict=True)) for row in timeline["rows"]]
        assert any(row["layer"] == "net" and row["ref"] == net["id"] for row in timeline_rows)
        assert any(row["summary"] == "POST 127.0.0.1/echo" for row in timeline_rows)

        security_latest = client.get(f"/vms/{session_id}/security/latest?limit=50", timeout=30)
        assert any(row["event_id"] == event_id for row in security_latest)
        latest_row = next(
            row
            for row in security_latest
            if row["event_id"] == event_id
            and row["rule_id"] == "corp.rules.allow_ironbank_mock_http"
        )
        assert latest_row["event_type"] == "http.request"
        assert latest_row["rule_id"] == "corp.rules.allow_ironbank_mock_http"
        assert latest_row["rule_action"] == "allow"
        assert latest_row["detection_level"] == "none"

        security_status = client.get(f"/vms/{session_id}/security/status", timeout=30)
        assert security_status["total"] >= len(security_rows)
        by_action = {row["rule_action"]: row["count"] for row in security_status["by_action"]}
        by_event_type = {
            row["event_type"]: row["count"] for row in security_status["by_event_type"]
        }
        assert by_action["allow"] >= 1
        assert by_event_type["http.request"] >= 1

        vm_list = client.get("/vms/list", timeout=30)
        sandboxes = vm_list["sandboxes"] if isinstance(vm_list, dict) else vm_list
        session_stats = next(row for row in sandboxes if row["id"] == session_id)
        assert session_stats["total_requests"] >= 1
        assert session_stats["allowed_requests"] >= 1
        assert session_stats["denied_requests"] == 0

        service_log = (service.tmp_dir / "service.log").read_text(encoding="utf-8")
        gateway_log = (gateway.run_dir / "gateway.log").read_text(encoding="utf-8")
        assert "handle_exec" in service_log or "exec" in service_log
        assert "gateway.proxy.ok" in gateway_log
        assert f"/vms/{session_id}/inspect" in gateway_log
        assert gateway_body == ""
    finally:
        stop_process(mock_proc)
        if client is not None:
            try:
                client.delete(f"/vms/{session_id}/delete", timeout=60)
            except Exception:
                pass
        if gateway is not None:
            gateway.stop()
        service.stop()
        if old_corp_config is None:
            os.environ.pop("CAPSEM_CORP_CONFIG", None)
        else:
            os.environ["CAPSEM_CORP_CONFIG"] = old_corp_config


def test_denied_http_request_pays_full_ledger_debt_blackbox() -> None:
    assert MOCK_SERVER_BINARY.exists(), f"{MOCK_SERVER_BINARY} missing"
    assert ASSETS_DIR.exists(), f"{ASSETS_DIR} missing; build VM assets before Ironbank"
    assert PROFILES_DIR.exists(), f"{PROFILES_DIR} missing; materialize profile config"

    service = ServiceInstance()
    gateway: GatewayInstance | None = None
    mock_proc = None
    client = None
    session_id = vm_name("ironbank-http-deny")
    nonce = uuid.uuid4().hex
    old_corp_config = os.environ.get("CAPSEM_CORP_CONFIG")
    try:
        mock_proc, ready = start_mock_server(
            request_log=service.tmp_dir / "upstream-http-deny-transcript.jsonl"
        )
        corp_path = service.tmp_dir / "corp.toml"
        corp_path.write_text(
            textwrap.dedent(
                """
                refresh_policy = "24h"

                [settings."vm.resources.log_bodies"]
                value = true
                modified = "2026-06-14T00:00:00Z"

                [settings."vm.resources.max_body_capture"]
                value = 8192
                modified = "2026-06-14T00:00:00Z"

                [settings."security.web.http_upstream_ports"]
                value = [80, 3713, 8080]
                modified = "2026-06-14T00:00:00Z"

                [corp.rules.block_ironbank_mock_http]
                name = "block_ironbank_mock_http"
                action = "block"
                priority = -100
                detection_level = "high"
                reason = "Block the hermetic Ironbank HTTP denial fixture."
                match = 'http.host == "127.0.0.1" && tcp.port == "3713" && http.path == "/deny-target"'
                """
            ).strip()
            + "\n",
            encoding="utf-8",
        )
        os.environ["CAPSEM_CORP_CONFIG"] = str(corp_path)
        service.start()
        client = service.client()
        gateway = GatewayInstance(uds_path=service.uds_path)
        gateway.start()
        gateway_client = TcpHttpClient(gateway.base_url, gateway.token)

        create = client.post(
            "/vms/create",
            {
                "name": session_id,
                "profile_id": CODE_PROFILE_ID,
                "ram_mb": DEFAULT_RAM_MB,
                "cpus": DEFAULT_CPUS,
                "env": {"CAPSEM_MOCK_SERVER_BASE_URL": ready["base_url"]},
            },
            timeout=90,
        )
        assert create is not None
        assert create.get("id") == session_id or create.get("name") == session_id
        assert wait_exec_ready(client, session_id, timeout=EXEC_READY_TIMEOUT)

        script = textwrap.dedent(
            f"""
            import json
            import urllib.error
            import urllib.request

            payload = {{"kind": "ironbank_http_denied_json", "nonce": {json.dumps(nonce)}}}
            body = json.dumps(payload, sort_keys=True, separators=(",", ":")).encode()
            req = urllib.request.Request(
                {json.dumps(ready["base_url"].rstrip("/") + "/deny-target?case=blocked-json")},
                data=body,
                method="POST",
                headers={{
                    "content-type": "application/json",
                    "user-agent": "capsem-ironbank-http-deny/1",
                    "x-ironbank-nonce": {json.dumps(nonce)},
                }},
            )
            try:
                urllib.request.urlopen(req, timeout=30)
                raise AssertionError("blocked HTTP request unexpectedly reached upstream")
            except urllib.error.HTTPError as error:
                response_body = error.read().decode()
                result = {{
                    "status": error.code,
                    "body": response_body,
                    "request_body": body.decode(),
                    "nonce": {json.dumps(nonce)},
                }}
            print("IRONBANK_HTTP_DENY_RESULT=" + json.dumps(result, sort_keys=True))
            """
        ).strip()
        upload = client.post_bytes(
            f"/vms/{session_id}/files/content?path=ironbank-http-deny.py",
            script.encode(),
            timeout=30,
        )
        assert upload is not None
        assert upload["success"] is True

        exec_resp = client.post(
            f"/vms/{session_id}/exec",
            {"command": "python3 /root/ironbank-http-deny.py", "timeout_secs": 120},
            timeout=150,
        )
        assert exec_resp is not None
        assert exec_resp["exit_code"] == 0, exec_resp
        result = _one_json_line(
            exec_resp.get("stdout") or "", "IRONBANK_HTTP_DENY_RESULT="
        )
        assert result["status"] == 403
        assert (
            result["body"]
            == "capsem: HTTP request blocked by security rule: corp.rules.block_ironbank_mock_http\n"
        )
        assert result["nonce"] == nonce
        assert nonce in result["request_body"]

        request_log_path = Path(ready["request_log"])
        upstream_text = (
            request_log_path.read_text(encoding="utf-8") if request_log_path.exists() else ""
        )
        upstream_records = [
            json.loads(line) for line in upstream_text.splitlines() if line.strip()
        ]
        assert [row for row in upstream_records if row["path"] == "/deny-target"] == []

        with closing(_connect_session_db(service, session_id)) as conn:
            assert _table_columns(conn, "net_events") == EXPECTED_NET_COLUMNS
            assert _table_columns(conn, "security_rule_events") == EXPECTED_SECURITY_COLUMNS
            rows = conn.execute(
                """
                SELECT * FROM net_events
                WHERE method = 'POST' AND path = '/deny-target' AND query = 'case=blocked-json'
                ORDER BY id
                """
            ).fetchall()
            assert len(rows) == 1, [dict(row) for row in rows]
            net = dict(rows[0])
            event_id = _event_id(net["event_id"])
            assert net["domain"] == "127.0.0.1"
            assert net["port"] == 3713
            assert net["decision"] == "denied"
            assert net["status_code"] == 403
            assert net["bytes_sent"] == len(result["request_body"])
            assert net["bytes_received"] == len(result["body"])
            assert isinstance(net["duration_ms"], int)
            assert net["duration_ms"] >= 0
            assert net["matched_rule"] == "corp.rules.block_ironbank_mock_http"
            assert net["policy_action"] == "block"
            assert net["policy_rule"] == "corp.rules.block_ironbank_mock_http"
            assert net["credential_ref"] is None
            assert net["conn_type"] == "http-mitm"
            assert nonce not in net["request_headers"]
            assert re.search(
                r"x-ironbank-nonce: hash:[0-9a-f]{12}",
                net["request_headers"].lower(),
            )
            assert net["request_body_preview"] == result["request_body"]
            assert net["response_body_preview"] == result["body"]
            assert isinstance(net["trace_id"], str) and net["trace_id"]

            security_rows = conn.execute(
                """
                SELECT * FROM security_rule_events
                WHERE event_id = ? AND event_type = 'http.request'
                ORDER BY id
                """,
                (event_id,),
            ).fetchall()
            assert len(security_rows) >= 1, [dict(row) for row in security_rows]
            block_rule = next(
                row
                for row in security_rows
                if row["rule_id"] == "corp.rules.block_ironbank_mock_http"
            )
            assert block_rule["rule_action"] == "block"
            assert block_rule["detection_level"] == "high"
            assert block_rule["trace_id"] == net["trace_id"]
            event_json = json.loads(block_rule["event_json"])
            assert event_json["event_type"] == "http.request"
            assert event_json["http"]["host"] == "127.0.0.1"
            assert event_json["http"]["method"] == "POST"
            assert event_json["http"]["path"] == "/deny-target"
            assert event_json["http"]["query"] == "case=blocked-json"
            assert event_json["http"]["status"] == "403"
            assert event_json["http"]["body"] == result["request_body"]
            assert event_json["tcp"]["port"] == "3713"
            assert event_json["ip"]["value"] == "127.0.0.1"
            assert event_json["ip"]["version"] == "4"

        uds_rows = _query_rows(
            client,
            session_id,
            """
            SELECT event_id, domain, port, method, path, query, status_code, decision,
                   bytes_sent, bytes_received, matched_rule, request_body_preview,
                   response_body_preview, conn_type, trace_id
            FROM net_events
            WHERE event_id = '%s'
            """
            % event_id,
        )
        assert len(uds_rows) == 1
        assert uds_rows[0]["event_id"] == event_id
        assert uds_rows[0]["decision"] == "denied"
        assert uds_rows[0]["request_body_preview"] == result["request_body"]
        assert uds_rows[0]["response_body_preview"] == result["body"]

        gateway_rows = gateway_client.post(
            f"/vms/{session_id}/inspect",
            {
                "sql": (
                    "SELECT event_id, method, path, status_code, decision, trace_id "
                    f"FROM net_events WHERE event_id = '{event_id}'"
                )
            },
            timeout=30,
        )
        assert gateway_rows["rows"] == [[event_id, "POST", "/deny-target", 403, "denied", net["trace_id"]]]

        security_latest = client.get(f"/vms/{session_id}/security/latest?limit=50", timeout=30)
        latest_row = next(
            row
            for row in security_latest
            if row["event_id"] == event_id
            and row["rule_id"] == "corp.rules.block_ironbank_mock_http"
        )
        assert latest_row["event_type"] == "http.request"
        assert latest_row["rule_action"] == "block"
        assert latest_row["detection_level"] == "high"

        security_status = client.get(f"/vms/{session_id}/security/status", timeout=30)
        by_action = {row["rule_action"]: row["count"] for row in security_status["by_action"]}
        by_event_type = {
            row["event_type"]: row["count"] for row in security_status["by_event_type"]
        }
        assert by_action["block"] >= 1
        assert by_event_type["http.request"] >= 1

        vm_list = client.get("/vms/list", timeout=30)
        sandboxes = vm_list["sandboxes"] if isinstance(vm_list, dict) else vm_list
        session_stats = next(row for row in sandboxes if row["id"] == session_id)
        assert session_stats["total_requests"] >= 1
        assert session_stats["denied_requests"] >= 1

        service_log = (service.tmp_dir / "service.log").read_text(encoding="utf-8")
        gateway_log = (gateway.run_dir / "gateway.log").read_text(encoding="utf-8")
        assert "handle_exec" in service_log or "exec" in service_log
        assert "gateway.proxy.ok" in gateway_log
        assert f"/vms/{session_id}/inspect" in gateway_log
    finally:
        stop_process(mock_proc)
        if client is not None:
            try:
                client.delete(f"/vms/{session_id}/delete", timeout=60)
            except Exception:
                pass
        if gateway is not None:
            gateway.stop()
        service.stop()
        if old_corp_config is None:
            os.environ.pop("CAPSEM_CORP_CONFIG", None)
        else:
            os.environ["CAPSEM_CORP_CONFIG"] = old_corp_config
