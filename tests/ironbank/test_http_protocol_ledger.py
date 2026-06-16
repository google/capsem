"""Ironbank HTTP protocol ledger contract tests."""

from __future__ import annotations

from contextlib import closing
import json
import os
from pathlib import Path
import re
import sqlite3
import textwrap
import time
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

EXPECTED_SECURITY_ASK_COLUMNS = {
    "id",
    "timestamp_unix_ms",
    "ask_id",
    "event_id",
    "event_type",
    "rule_id",
    "rule_name",
    "status",
    "rule_json",
    "event_json",
    "resolver",
    "reason",
    "trace_id",
}

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


def _credential_ref(value: object) -> str:
    assert isinstance(value, str)
    assert re.fullmatch(r"credential:blake3:[0-9a-f]{64}", value), value
    return value


def _eventually(fetch, predicate, *, timeout_s: float = 20.0, interval_s: float = 0.25):
    deadline = time.monotonic() + timeout_s
    last = None
    while time.monotonic() < deadline:
        last = fetch()
        if predicate(last):
            return last
        time.sleep(interval_s)
    assert predicate(last), f"condition not met before timeout; last={last!r}"
    return last


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


def test_http_body_handling_matrix_pays_full_ledger_debt_blackbox() -> None:
    assert MOCK_SERVER_BINARY.exists(), f"{MOCK_SERVER_BINARY} missing"
    assert ASSETS_DIR.exists(), f"{ASSETS_DIR} missing; build VM assets before Ironbank"
    assert PROFILES_DIR.exists(), f"{PROFILES_DIR} missing; materialize profile config"

    service = ServiceInstance()
    gateway: GatewayInstance | None = None
    mock_proc = None
    client = None
    session_id = vm_name("ironbank-http-body")
    nonce = uuid.uuid4().hex
    old_corp_config = os.environ.get("CAPSEM_CORP_CONFIG")
    try:
        mock_proc, ready = start_mock_server(
            request_log=service.tmp_dir / "upstream-http-body-transcript.jsonl"
        )
        corp_path = service.tmp_dir / "corp.toml"
        corp_path.write_text(
            textwrap.dedent(
                f"""
                refresh_policy = "24h"

                [network.dns]
                upstreams = [{json.dumps(ready["dns_udp_addr"])}]

                [network.upstream_overrides."daily-cloudcode-pa.googleapis.com:443"]
                dial = {json.dumps(ready["http_addr"])}
                protocol = "http"

                [settings."vm.resources.log_bodies"]
                value = true
                modified = "2026-06-14T00:00:00Z"

                [settings."vm.resources.max_body_capture"]
                value = 128
                modified = "2026-06-14T00:00:00Z"

                [settings."security.web.http_upstream_ports"]
                value = [80, 3713, 8080]
                modified = "2026-06-14T00:00:00Z"

                [corp.rules.allow_ironbank_mock_http_body_matrix]
                name = "allow_ironbank_mock_http_body_matrix"
                action = "allow"
                priority = -100
                detection_level = "informational"
                reason = "Allow hermetic Ironbank HTTP body-handling fixtures."
                match = '(http.host == "127.0.0.1" && tcp.port == "3713" && (http.path == "/gzip/10kb" || http.path == "/chunked" || http.path == "/sse/model" || http.path == "/bytes/10kb")) || (http.host == "daily-cloudcode-pa.googleapis.com" && tcp.port == "443" && http.path == "/tiny")'
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
                "env": {
                    "CAPSEM_MOCK_SERVER_BASE_URL": ready["base_url"],
                },
            },
            timeout=90,
        )
        assert create is not None
        assert create.get("id") == session_id or create.get("name") == session_id
        assert wait_exec_ready(client, session_id, timeout=EXEC_READY_TIMEOUT)

        script = textwrap.dedent(
            f"""
            import json
            import ssl
            import urllib.request

            base = {json.dumps(ready["base_url"].rstrip("/"))}
            https_base = "https://daily-cloudcode-pa.googleapis.com"
            nonce = {json.dumps(nonce)}

            def fetch(name, url, *, insecure_tls=False):
                request = urllib.request.Request(
                    url,
                    method="GET",
                    headers={{
                        "user-agent": "capsem-ironbank-http-body/1",
                        "x-ironbank-nonce": nonce,
                        "x-ironbank-case": name,
                    }},
                )
                context = ssl._create_unverified_context() if insecure_tls else None
                with urllib.request.urlopen(request, timeout=30, context=context) as response:
                    raw = response.read()
                    return {{
                        "name": name,
                        "status": response.status,
                        "content_type": response.headers.get("content-type"),
                        "content_encoding": response.headers.get("content-encoding"),
                        "raw_len": len(raw),
                        "decoded_len": len(raw),
                        "decoded_prefix": raw[:48].decode("utf-8", "replace"),
                    }}

            cases = [
                fetch("gzip", base + "/gzip/10kb?case=gzip"),
                fetch("chunked", base + "/chunked?case=chunked"),
                fetch("sse", base + "/sse/model?case=sse"),
                fetch("truncated_preview", base + "/bytes/10kb?case=truncated-preview"),
                fetch("https", https_base + "/tiny?case=https", insecure_tls=True),
            ]
            print("IRONBANK_HTTP_BODY_MATRIX=" + json.dumps({{
                "nonce": nonce,
                "cases": cases,
            }}, sort_keys=True))
            """
        ).strip()
        upload = client.post_bytes(
            f"/vms/{session_id}/files/content?path=ironbank-http-body.py",
            script.encode(),
            timeout=30,
        )
        assert upload is not None
        assert upload["success"] is True

        exec_resp = client.post(
            f"/vms/{session_id}/exec",
            {"command": "python3 /root/ironbank-http-body.py", "timeout_secs": 120},
            timeout=150,
        )
        assert exec_resp is not None
        assert exec_resp["exit_code"] == 0, exec_resp
        result = _one_json_line(
            exec_resp.get("stdout") or "", "IRONBANK_HTTP_BODY_MATRIX="
        )
        assert result["nonce"] == nonce
        by_name = {case["name"]: case for case in result["cases"]}
        assert set(by_name) == {"gzip", "chunked", "sse", "truncated_preview", "https"}
        assert by_name["gzip"]["status"] == 200
        assert by_name["gzip"]["content_encoding"] is None
        assert by_name["gzip"]["raw_len"] == by_name["gzip"]["decoded_len"]
        assert by_name["gzip"]["decoded_len"] == 10 * 1024
        assert by_name["gzip"]["decoded_prefix"].startswith("abcdefghijklmnopqrstuvwxyz")
        assert by_name["chunked"]["decoded_prefix"] == "chunk-0\nchunk-1\nchunk-2\nchunk-3\n"
        assert by_name["sse"]["content_type"].startswith("text/event-stream")
        assert "event: model.delta" in by_name["sse"]["decoded_prefix"]
        assert by_name["truncated_preview"]["decoded_len"] == 10 * 1024
        assert by_name["https"]["decoded_prefix"] == "capsem-mock-server:tiny\n"

        request_log_path = Path(ready["request_log"])
        upstream_text = (
            request_log_path.read_text(encoding="utf-8") if request_log_path.exists() else ""
        )
        upstream_records = [
            json.loads(line) for line in upstream_text.splitlines() if line.strip()
        ]
        expected_upstream = {
            ("/gzip/10kb", "case=gzip"),
            ("/chunked", "case=chunked"),
            ("/sse/model", "case=sse"),
            ("/bytes/10kb", "case=truncated-preview"),
            ("/tiny", "case=https"),
        }
        observed_upstream = {
            (row["path"], row["query"])
            for row in upstream_records
            if isinstance(row.get("headers"), dict)
            and row["headers"].get("x-ironbank-nonce") == nonce
        }
        assert expected_upstream <= observed_upstream, upstream_records

        with closing(_connect_session_db(service, session_id)) as conn:
            assert _table_columns(conn, "net_events") == EXPECTED_NET_COLUMNS
            assert _table_columns(conn, "security_rule_events") == EXPECTED_SECURITY_COLUMNS
            rows = _eventually(
                lambda: conn.execute(
                    """
                    SELECT *
                    FROM net_events
                    WHERE method = 'GET'
                      AND query IN (
                        'case=gzip',
                        'case=chunked',
                        'case=sse',
                        'case=truncated-preview',
                        'case=https'
                      )
                    ORDER BY id
                    """
                ).fetchall(),
                lambda found: len(found) == 5,
            )
            nets = {row["query"]: dict(row) for row in rows}
            assert set(nets) == {
                "case=gzip",
                "case=chunked",
                "case=sse",
                "case=truncated-preview",
                "case=https",
            }
            expected_paths = {
                "case=gzip": "/gzip/10kb",
                "case=chunked": "/chunked",
                "case=sse": "/sse/model",
                "case=truncated-preview": "/bytes/10kb",
                "case=https": "/tiny",
            }
            for query, net in nets.items():
                event_id = _event_id(net["event_id"])
                expected_host = (
                    "daily-cloudcode-pa.googleapis.com"
                    if query == "case=https"
                    else "127.0.0.1"
                )
                assert net["domain"] == expected_host
                assert net["port"] == (443 if query == "case=https" else 3713)
                assert net["method"] == "GET"
                assert net["path"] == expected_paths[query]
                assert net["status_code"] == 200
                assert net["decision"] == "allowed"
                assert net["matched_rule"] == "corp.rules.allow_ironbank_mock_http_body_matrix"
                assert net["policy_action"] == "allow"
                assert net["policy_rule"] == "corp.rules.allow_ironbank_mock_http_body_matrix"
                assert net["credential_ref"] is None
                assert net["conn_type"] == ("https-mitm" if query == "case=https" else "http-mitm")
                assert nonce not in net["request_headers"]
                assert re.search(
                    r"x-ironbank-nonce: hash:[0-9a-f]{12}",
                    net["request_headers"].lower(),
                )
                assert isinstance(net["duration_ms"], int)
                assert net["duration_ms"] >= 0
                assert isinstance(net["trace_id"], str) and net["trace_id"]

                security_rows = conn.execute(
                    """
                    SELECT *
                    FROM security_rule_events
                    WHERE event_id = ? AND event_type = 'http.request'
                    ORDER BY id
                    """,
                    (event_id,),
                ).fetchall()
                body_rule = next(
                    row
                    for row in security_rows
                    if row["rule_id"] == "corp.rules.allow_ironbank_mock_http_body_matrix"
                )
                assert body_rule["rule_action"] == "allow"
                assert body_rule["detection_level"] == "informational"
                assert body_rule["trace_id"] == net["trace_id"]
                event_json = json.loads(body_rule["event_json"])
                assert event_json["event_type"] == "http.request"
                assert event_json["http"]["host"] == expected_host
                assert event_json["http"]["method"] == "GET"
                assert event_json["http"]["path"] == expected_paths[query]
                assert event_json["http"]["query"] == query
                assert event_json["http"]["status"] == "200"
                assert event_json["tcp"]["port"] == str(net["port"])
                if query == "case=https":
                    assert event_json["ip"] is None
                else:
                    assert event_json["ip"]["value"] == "127.0.0.1"
                    assert event_json["ip"]["version"] == "4"

            gzip_net = nets["case=gzip"]
            assert "content-encoding: gzip" not in (
                gzip_net["response_headers"] or ""
            ).lower()
            assert "content-length:" not in (
                gzip_net["response_headers"] or ""
            ).lower()
            assert gzip_net["bytes_received"] == 10 * 1024
            assert (gzip_net["response_body_preview"] or "").startswith(
                "abcdefghijklmnopqrstuvwxyz"
            )

            chunked_net = nets["case=chunked"]
            assert chunked_net["response_body_preview"] == "chunk-0\nchunk-1\nchunk-2\nchunk-3\n"

            sse_net = nets["case=sse"]
            assert "content-type: text/event-stream" in (
                sse_net["response_headers"] or ""
            ).lower()
            assert "event: model.delta" in (sse_net["response_body_preview"] or "")
            assert "event: model.tool_call" in (sse_net["response_body_preview"] or "")

            truncated_net = nets["case=truncated-preview"]
            assert truncated_net["bytes_received"] == 10 * 1024
            assert len(truncated_net["response_body_preview"] or "") <= 128
            assert (truncated_net["response_body_preview"] or "").startswith(
                "abcdefghijklmnopqrstuvwxyz"
            )

            https_net = nets["case=https"]
            assert https_net["response_body_preview"] == "capsem-mock-server:tiny\n"

            uds_rows = _query_rows(
                client,
                session_id,
                """
                SELECT event_id, path, query, status_code, decision, conn_type, trace_id
                FROM net_events
                WHERE query IN (
                  'case=gzip',
                  'case=chunked',
                  'case=sse',
                  'case=truncated-preview',
                  'case=https'
                )
                ORDER BY query
                """,
            )
            assert len(uds_rows) == 5
            assert {row["query"] for row in uds_rows} == set(nets)
            assert {row["decision"] for row in uds_rows} == {"allowed"}
            assert any(row["conn_type"] == "https-mitm" for row in uds_rows)

            gateway_rows = gateway_client.post(
                f"/vms/{session_id}/inspect",
                {
                    "sql": (
                        "SELECT path, query, status_code, decision, conn_type "
                        "FROM net_events WHERE query IN "
                        "('case=gzip','case=chunked','case=sse','case=truncated-preview','case=https') "
                        "ORDER BY query"
                    )
                },
                timeout=30,
            )
            assert gateway_rows["columns"] == [
                "path",
                "query",
                "status_code",
                "decision",
                "conn_type",
            ]
            assert len(gateway_rows["rows"]) == 5
            assert [row[2] for row in gateway_rows["rows"]] == [200, 200, 200, 200, 200]

            security_latest = client.get(f"/vms/{session_id}/security/latest?limit=100", timeout=30)
            latest_rows = [
                row
                for row in security_latest
                if row["rule_id"] == "corp.rules.allow_ironbank_mock_http_body_matrix"
            ]
            assert len(latest_rows) >= 5
            assert {row["event_type"] for row in latest_rows} == {"http.request"}
            assert {row["rule_action"] for row in latest_rows} == {"allow"}
            assert {row["detection_level"] for row in latest_rows} == {"informational"}

            security_status = client.get(f"/vms/{session_id}/security/status", timeout=30)
            by_action = {row["rule_action"]: row["count"] for row in security_status["by_action"]}
            by_event_type = {
                row["event_type"]: row["count"] for row in security_status["by_event_type"]
            }
            assert by_action["allow"] >= 5
            assert by_event_type["http.request"] >= 5

        vm_list = client.get("/vms/list", timeout=30)
        sandboxes = vm_list["sandboxes"] if isinstance(vm_list, dict) else vm_list
        session_stats = next(row for row in sandboxes if row["id"] == session_id)
        assert session_stats["total_requests"] >= 5
        assert session_stats["allowed_requests"] >= 5
        assert session_stats["denied_requests"] == 0

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


def test_brokered_http_rewrite_pays_full_ledger_debt_blackbox() -> None:
    assert MOCK_SERVER_BINARY.exists(), f"{MOCK_SERVER_BINARY} missing"
    assert ASSETS_DIR.exists(), f"{ASSETS_DIR} missing; build VM assets before Ironbank"
    assert PROFILES_DIR.exists(), f"{PROFILES_DIR} missing; materialize profile config"

    service = ServiceInstance()
    gateway: GatewayInstance | None = None
    mock_proc = None
    client = None
    session_id = vm_name("ironbank-http-rewrite")
    nonce = uuid.uuid4().hex
    old_corp_config = os.environ.get("CAPSEM_CORP_CONFIG")
    try:
        mock_proc, ready = start_mock_server(
            request_log=service.tmp_dir / "upstream-http-rewrite-transcript.jsonl"
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

                [corp.rules.allow_ironbank_mock_http_rewrite]
                name = "allow_ironbank_mock_http_rewrite"
                action = "allow"
                priority = -100
                detection_level = "informational"
                reason = "Allow the hermetic Ironbank credential-broker rewrite fixture."
                match = 'http.host == "127.0.0.1" && tcp.port == "3713" && (http.path == "/oauth/token" || http.path == "/echo")'
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

        capture_script = textwrap.dedent(
            f"""
            import json
            import urllib.parse
            import urllib.request

            token_req = urllib.request.Request(
                {json.dumps(ready["base_url"].rstrip("/") + "/oauth/token")},
                data=urllib.parse.urlencode({{"code": "capsem_test_oauth_code_rewrite_{nonce}"}}).encode(),
                method="POST",
                headers={{
                    "content-type": "application/x-www-form-urlencoded",
                    "user-agent": "capsem-ironbank-http-rewrite-capture/1",
                    "x-ironbank-nonce": {json.dumps(nonce)},
                }},
            )
            with urllib.request.urlopen(token_req, timeout=30) as response:
                token_body = json.loads(response.read().decode())
            print("IRONBANK_HTTP_REWRITE_CAPTURE=" + json.dumps({{
                "status": "captured",
                "kind": token_body["kind"],
                "nonce": {json.dumps(nonce)},
            }}, sort_keys=True))
            """
        ).strip()
        upload = client.post_bytes(
            f"/vms/{session_id}/files/content?path=ironbank-http-rewrite-capture.py",
            capture_script.encode(),
            timeout=30,
        )
        assert upload is not None
        assert upload["success"] is True
        capture_exec = client.post(
            f"/vms/{session_id}/exec",
            {
                "command": "python3 /root/ironbank-http-rewrite-capture.py",
                "timeout_secs": 120,
            },
            timeout=150,
        )
        assert capture_exec is not None
        assert capture_exec["exit_code"] == 0, capture_exec
        capture_result = _one_json_line(
            capture_exec.get("stdout") or "", "IRONBANK_HTTP_REWRITE_CAPTURE="
        )
        assert capture_result == {
            "kind": "synthetic_oauth_token_fixture",
            "nonce": nonce,
            "status": "captured",
        }

        with closing(_connect_session_db(service, session_id)) as conn:
            token_rows = _eventually(
                lambda: conn.execute(
                    """
                    SELECT *
                    FROM net_events
                    WHERE path = '/oauth/token'
                    ORDER BY id
                    """
                ).fetchall(),
                lambda rows: len(rows) == 1 and rows[0]["credential_ref"] is not None,
            )
            token_net = dict(token_rows[0])
            _event_id(token_net["event_id"])
            _credential_ref(token_net["credential_ref"])
            assert token_net["domain"] == "127.0.0.1"
            assert token_net["port"] == 3713
            assert token_net["method"] == "POST"
            assert token_net["status_code"] == 200
            assert token_net["decision"] == "allowed"
            assert token_net["matched_rule"] == "corp.rules.allow_ironbank_mock_http_rewrite"
            assert token_net["policy_action"] == "allow"
            assert token_net["policy_rule"] == "corp.rules.allow_ironbank_mock_http_rewrite"
            assert token_net["conn_type"] == "http-mitm"
            assert "capsem_test_oauth_code_rewrite_" not in (
                token_net["request_body_preview"] or ""
            )
            assert "capsem_test_oauth_access_" not in (
                token_net["response_body_preview"] or ""
            )
            assert "capsem_test_oauth_refresh_" not in (
                token_net["response_body_preview"] or ""
            )
            assert "credential:blake3:" in (token_net["request_body_preview"] or "")
            assert "credential:blake3:" in (token_net["response_body_preview"] or "")
            response_access_token_refs = _eventually(
                lambda: conn.execute(
                    """
                    SELECT *
                    FROM substitution_events
                    WHERE source = 'http.body.response.$.access_token'
                      AND outcome = 'captured'
                    ORDER BY id
                    """
                ).fetchall(),
                lambda rows: len(rows) == 1,
            )
            credential_ref = _credential_ref(response_access_token_refs[0]["substitution_ref"])

        replay_script = textwrap.dedent(
            f"""
            import json
            import urllib.parse
            import urllib.request

            cfg = {{
                "credential_ref": {json.dumps(credential_ref)},
                "echo_url": {json.dumps(ready["base_url"].rstrip("/") + "/echo")},
                "nonce": {json.dumps(nonce)},
            }}
            header_req = urllib.request.Request(
                cfg["echo_url"],
                data=b"broker header rewrite",
                method="POST",
                headers={{
                    "authorization": "Bearer " + cfg["credential_ref"],
                    "content-type": "text/plain",
                    "user-agent": "capsem-ironbank-http-rewrite-header/1",
                    "x-ironbank-nonce": cfg["nonce"],
                }},
            )
            with urllib.request.urlopen(header_req, timeout=30) as response:
                header_echo = json.loads(response.read().decode())

            query_url = cfg["echo_url"] + "?access_token=" + urllib.parse.quote(
                cfg["credential_ref"],
                safe="",
            )
            query_req = urllib.request.Request(
                query_url,
                data=b"broker query rewrite",
                method="POST",
                headers={{
                    "content-type": "text/plain",
                    "user-agent": "capsem-ironbank-http-rewrite-query/1",
                    "x-ironbank-nonce": cfg["nonce"],
                }},
            )
            with urllib.request.urlopen(query_req, timeout=30) as response:
                query_echo = json.loads(response.read().decode())

            print("IRONBANK_HTTP_REWRITE_REPLAY=" + json.dumps({{
                "header_has_authorization": header_echo["has_authorization"],
                "header_authorization_is_broker_ref": header_echo["authorization_is_broker_ref"],
                "query_has_access_token": query_echo["query_has_access_token"],
                "query_has_broker_ref": query_echo["query_has_broker_ref"],
                "nonce": cfg["nonce"],
            }}, sort_keys=True))
            """
        ).strip()
        upload = client.post_bytes(
            f"/vms/{session_id}/files/content?path=ironbank-http-rewrite-replay.py",
            replay_script.encode(),
            timeout=30,
        )
        assert upload is not None
        assert upload["success"] is True
        replay_exec = client.post(
            f"/vms/{session_id}/exec",
            {
                "command": "python3 /root/ironbank-http-rewrite-replay.py",
                "timeout_secs": 120,
            },
            timeout=150,
        )
        assert replay_exec is not None
        assert replay_exec["exit_code"] == 0, replay_exec
        replay_result = _one_json_line(
            replay_exec.get("stdout") or "", "IRONBANK_HTTP_REWRITE_REPLAY="
        )
        assert replay_result == {
            "header_authorization_is_broker_ref": False,
            "header_has_authorization": True,
            "nonce": nonce,
            "query_has_access_token": True,
            "query_has_broker_ref": False,
        }

        request_log_path = Path(ready["request_log"])
        upstream_text = (
            request_log_path.read_text(encoding="utf-8") if request_log_path.exists() else ""
        )
        upstream_records = [
            json.loads(line) for line in upstream_text.splitlines() if line.strip()
        ]
        upstream_echo = [row for row in upstream_records if row["path"] == "/echo"]
        assert len(upstream_echo) == 2, upstream_records
        header_upstream = next(row for row in upstream_echo if row["query"] == "")
        query_upstream = next(row for row in upstream_echo if "access_token=" in row["query"])
        assert credential_ref not in header_upstream["headers"].get("authorization", "")
        assert "credential:blake3:" not in header_upstream["headers"].get("authorization", "")
        assert header_upstream["headers"]["authorization"].startswith("Bearer capsem_test_")
        assert credential_ref not in query_upstream["query"]
        assert "credential:blake3:" not in query_upstream["query"]
        assert "access_token=capsem_test_" in query_upstream["query"]

        with closing(_connect_session_db(service, session_id)) as conn:
            assert _table_columns(conn, "net_events") == EXPECTED_NET_COLUMNS
            assert _table_columns(conn, "security_rule_events") == EXPECTED_SECURITY_COLUMNS
            assert _table_columns(conn, "substitution_events") == EXPECTED_SUBSTITUTION_COLUMNS

            echo_rows = _eventually(
                lambda: conn.execute(
                    """
                    SELECT *
                    FROM net_events
                    WHERE path = '/echo'
                    ORDER BY id
                    """
                ).fetchall(),
                lambda rows: len(rows) == 2,
            )
            header_net = dict(next(row for row in echo_rows if not row["query"]))
            query_net = dict(next(row for row in echo_rows if row["query"]))
            for net in (header_net, query_net):
                event_id = _event_id(net["event_id"])
                assert net["domain"] == "127.0.0.1"
                assert net["port"] == 3713
                assert net["method"] == "POST"
                assert net["status_code"] == 200
                assert net["decision"] == "allowed"
                assert net["matched_rule"] == "corp.rules.allow_ironbank_mock_http_rewrite"
                assert net["policy_action"] == "allow"
                assert net["policy_rule"] == "corp.rules.allow_ironbank_mock_http_rewrite"
                assert net["credential_ref"] == credential_ref
                assert net["conn_type"] == "http-mitm"
                assert isinstance(net["trace_id"], str) and net["trace_id"]
                assert "capsem_test_oauth_access_" not in (net["request_headers"] or "")
                assert "capsem_test_oauth_access_" not in (net["query"] or "")
                assert "credential:blake3:" not in (net["request_headers"] or "")
                assert "authorization: hash:" in (net["request_headers"] or "").lower() or net is query_net
                assert "credential:blake3:" not in (net["response_body_preview"] or "")
                response_preview = json.loads(net["response_body_preview"])
                assert response_preview["path"] == "/echo"
                assert response_preview["authorization_is_broker_ref"] is False
                if net is header_net:
                    assert response_preview["has_authorization"] is True
                    assert response_preview["query_has_access_token"] is False
                    assert response_preview["query_has_broker_ref"] is False
                else:
                    assert response_preview["has_authorization"] is False
                    assert response_preview["query_has_access_token"] is True
                    assert response_preview["query_has_broker_ref"] is False

                security_rows = conn.execute(
                    """
                    SELECT *
                    FROM security_rule_events
                    WHERE event_id = ? AND event_type = 'http.request'
                    ORDER BY id
                    """,
                    (event_id,),
                ).fetchall()
                assert security_rows, event_id
                rewrite_rule = next(
                    row
                    for row in security_rows
                    if row["rule_id"] == "corp.rules.allow_ironbank_mock_http_rewrite"
                )
                assert rewrite_rule["rule_action"] == "allow"
                assert rewrite_rule["detection_level"] == "informational"
                assert rewrite_rule["trace_id"] == net["trace_id"]
                event_json = json.loads(rewrite_rule["event_json"])
                assert event_json["event_type"] == "http.request"
                assert event_json["http"]["host"] == "127.0.0.1"
                assert event_json["http"]["path"] == "/echo"
                assert event_json["tcp"]["port"] == "3713"
                assert event_json["ip"]["value"] == "127.0.0.1"

            assert header_net["query"] in (None, "")
            assert query_net["query"].startswith("access_token=")
            assert credential_ref not in query_net["query"]
            assert "capsem_test_oauth_access_" not in query_net["query"]

            substitutions = conn.execute(
                """
                SELECT *
                FROM substitution_events
                WHERE substitution_ref = ?
                ORDER BY id
                """,
                (credential_ref,),
            ).fetchall()
            assert substitutions
            outcomes = {row["outcome"] for row in substitutions}
            assert {"captured", "brokered", "injected"} <= outcomes
            assert all(row["material_class"] == "credential" for row in substitutions)
            assert all(row["algorithm"] == "blake3" for row in substitutions)
            assert all(row["substitution_ref"] == credential_ref for row in substitutions)
            assert all(row["provider"] == "google" for row in substitutions)
            assert all(row["confidence"] is None for row in substitutions)
            assert all(row["trace_id"] for row in substitutions)
            sources_by_outcome = {
                outcome: {
                    row["source"] for row in substitutions if row["outcome"] == outcome
                }
                for outcome in outcomes
            }
            assert "http.body.response.$.access_token" in sources_by_outcome["captured"]
            assert "http.body.response.$.access_token" in sources_by_outcome["brokered"]
            assert "http.header.authorization" in sources_by_outcome["injected"]
            assert "http.query.access_token" in sources_by_outcome["injected"]

            uds_rows = _query_rows(
                client,
                session_id,
                """
                SELECT event_id, path, query, status_code, decision, credential_ref,
                       request_headers, response_body_preview, trace_id
                FROM net_events
                WHERE path = '/echo'
                ORDER BY id
                """,
            )
            assert len(uds_rows) == 2
            assert {row["credential_ref"] for row in uds_rows} == {credential_ref}
            assert all("capsem_test_oauth_access_" not in (row["request_headers"] or "") for row in uds_rows)
            assert all("credential:blake3:" not in (row["request_headers"] or "") for row in uds_rows)

            gateway_rows = gateway_client.post(
                f"/vms/{session_id}/inspect",
                {
                    "sql": (
                        "SELECT event_id, method, path, status_code, decision, credential_ref "
                        "FROM net_events WHERE path = '/echo' ORDER BY id"
                    )
                },
                timeout=30,
            )
            assert gateway_rows["columns"] == [
                "event_id",
                "method",
                "path",
                "status_code",
                "decision",
                "credential_ref",
            ]
            assert len(gateway_rows["rows"]) == 2
            assert {row[5] for row in gateway_rows["rows"]} == {credential_ref}

            broker_reload = client.post(
                f"/profiles/{CODE_PROFILE_ID}/plugins/credential_broker/credentials/reload",
                {},
                timeout=30,
            )
            assert broker_reload["plugin_id"] == "credential_broker"
            assert broker_reload["store"]["ready"] is True
            assert any(
                credential["credential_ref"] == credential_ref
                and credential["provider"] == "google"
                and credential["observed_count"] >= 1
                and credential["injected_count"] >= 2
                and credential["replay_available"] is True
                for credential in broker_reload["inventory"]
            ), broker_reload["inventory"]

            plugins = client.get(f"/profiles/{CODE_PROFILE_ID}/plugins/list", timeout=30)
            assert plugins is not None
            by_plugin = {plugin["id"]: plugin for plugin in plugins["plugins"]}
            broker_runtime = by_plugin["credential_broker"]["runtime"]
            assert broker_runtime["enabled"] is True
            assert broker_runtime["execution_count"] >= 3
            assert broker_runtime["applied_count"] >= 2
            assert broker_runtime["detection_count"] >= 2
            assert broker_runtime["total_duration_us"] >= broker_runtime["max_duration_us"]
            assert broker_runtime["rewrite_count"] >= 2
            assert any(
                credential["credential_ref"] == credential_ref
                and credential["provider"] == "google"
                and credential["observed_count"] >= 1
                and credential["injected_count"] >= 2
                and credential["replay_available"] is True
                for credential in broker_runtime["brokered_credentials"]
            ), (
                credential_ref,
                [
                    (
                        credential["provider"],
                        credential["credential_ref"][-12:],
                        credential["observed_count"],
                        credential["injected_count"],
                        credential["replay_available"],
                    )
                    for credential in broker_runtime["brokered_credentials"]
                ],
            )

            broker_info = client.get(
                f"/profiles/{CODE_PROFILE_ID}/plugins/credential_broker/credentials/info",
                timeout=30,
            )
            assert broker_info["plugin_id"] == "credential_broker"
            assert broker_info["store"]["ready"] is True
            assert any(
                credential["credential_ref"] == credential_ref
                and credential["provider"] == "google"
                and credential["observed_count"] >= 1
                and credential["injected_count"] >= 2
                and credential["replay_available"] is True
                for credential in broker_info["inventory"]
            ), broker_info["inventory"]

            security_latest = client.get(
                f"/vms/{session_id}/security/latest?limit=50",
                timeout=30,
            )
            latest_echo = [
                row
                for row in security_latest
                if row["rule_id"] == "corp.rules.allow_ironbank_mock_http_rewrite"
                and row["event_type"] == "http.request"
            ]
            assert len(latest_echo) >= 3
            assert {row["rule_action"] for row in latest_echo} == {"allow"}
            assert "informational" in {row["detection_level"] for row in latest_echo}

            security_status = client.get(f"/vms/{session_id}/security/status", timeout=30)
            by_action = {row["rule_action"]: row["count"] for row in security_status["by_action"]}
            by_event_type = {
                row["event_type"]: row["count"] for row in security_status["by_event_type"]
            }
            assert by_action["allow"] >= 3
            assert by_event_type["http.request"] >= 3

        service_log = (service.tmp_dir / "service.log").read_text(encoding="utf-8")
        gateway_log = (gateway.run_dir / "gateway.log").read_text(encoding="utf-8")
        assert "capsem_test_oauth_access_" not in service_log
        assert "capsem_test_oauth_refresh_" not in service_log
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


def test_asked_http_request_pays_full_ledger_debt_blackbox() -> None:
    assert MOCK_SERVER_BINARY.exists(), f"{MOCK_SERVER_BINARY} missing"
    assert ASSETS_DIR.exists(), f"{ASSETS_DIR} missing; build VM assets before Ironbank"
    assert PROFILES_DIR.exists(), f"{PROFILES_DIR} missing; materialize profile config"

    service = ServiceInstance()
    gateway: GatewayInstance | None = None
    mock_proc = None
    client = None
    session_id = vm_name("ironbank-http-ask")
    nonce = uuid.uuid4().hex
    old_corp_config = os.environ.get("CAPSEM_CORP_CONFIG")
    try:
        mock_proc, ready = start_mock_server(
            request_log=service.tmp_dir / "upstream-http-ask-transcript.jsonl"
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

                [corp.rules.ask_ironbank_mock_http]
                name = "ask_ironbank_mock_http"
                action = "ask"
                priority = -100
                detection_level = "medium"
                reason = "Require approval for the hermetic Ironbank HTTP ask fixture."
                match = 'http.host == "127.0.0.1" && tcp.port == "3713" && http.path == "/ask-target"'
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

            payload = {{"kind": "ironbank_http_asked_json", "nonce": {json.dumps(nonce)}}}
            body = json.dumps(payload, sort_keys=True, separators=(",", ":")).encode()
            req = urllib.request.Request(
                {json.dumps(ready["base_url"].rstrip("/") + "/ask-target?case=ask-json")},
                data=body,
                method="POST",
                headers={{
                    "content-type": "application/json",
                    "user-agent": "capsem-ironbank-http-ask/1",
                    "x-ironbank-nonce": {json.dumps(nonce)},
                }},
            )
            try:
                urllib.request.urlopen(req, timeout=30)
                raise AssertionError("pending ask HTTP request unexpectedly reached upstream")
            except urllib.error.HTTPError as error:
                response_body = error.read().decode()
                result = {{
                    "status": error.code,
                    "body": response_body,
                    "request_body": body.decode(),
                    "nonce": {json.dumps(nonce)},
                }}
            print("IRONBANK_HTTP_ASK_RESULT=" + json.dumps(result, sort_keys=True))
            """
        ).strip()
        upload = client.post_bytes(
            f"/vms/{session_id}/files/content?path=ironbank-http-ask.py",
            script.encode(),
            timeout=30,
        )
        assert upload is not None
        assert upload["success"] is True

        exec_resp = client.post(
            f"/vms/{session_id}/exec",
            {"command": "python3 /root/ironbank-http-ask.py", "timeout_secs": 120},
            timeout=150,
        )
        assert exec_resp is not None
        assert exec_resp["exit_code"] == 0, exec_resp
        result = _one_json_line(exec_resp.get("stdout") or "", "IRONBANK_HTTP_ASK_RESULT=")
        assert result["status"] == 403
        assert (
            result["body"]
            == "capsem: HTTP request requires approval by security rule: corp.rules.ask_ironbank_mock_http\n"
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
        assert [row for row in upstream_records if row["path"] == "/ask-target"] == []

        with closing(_connect_session_db(service, session_id)) as conn:
            assert _table_columns(conn, "net_events") == EXPECTED_NET_COLUMNS
            assert _table_columns(conn, "security_rule_events") == EXPECTED_SECURITY_COLUMNS
            assert _table_columns(conn, "security_ask_events") == EXPECTED_SECURITY_ASK_COLUMNS
            rows = conn.execute(
                """
                SELECT * FROM net_events
                WHERE method = 'POST' AND path = '/ask-target' AND query = 'case=ask-json'
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
            assert net["matched_rule"] == "corp.rules.ask_ironbank_mock_http"
            assert net["policy_action"] == "ask"
            assert net["policy_rule"] == "corp.rules.ask_ironbank_mock_http"
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
            ask_rule = next(
                row
                for row in security_rows
                if row["rule_id"] == "corp.rules.ask_ironbank_mock_http"
            )
            assert ask_rule["rule_action"] == "ask"
            assert ask_rule["detection_level"] == "medium"
            assert ask_rule["trace_id"] == net["trace_id"]
            event_json = json.loads(ask_rule["event_json"])
            assert event_json["event_type"] == "http.request"
            assert event_json["http"]["host"] == "127.0.0.1"
            assert event_json["http"]["method"] == "POST"
            assert event_json["http"]["path"] == "/ask-target"
            assert event_json["http"]["query"] == "case=ask-json"
            assert event_json["http"]["status"] == "403"
            assert event_json["http"]["body"] == result["request_body"]
            assert event_json["tcp"]["port"] == "3713"
            assert event_json["ip"]["value"] == "127.0.0.1"
            assert event_json["ip"]["version"] == "4"

            ask_rows = conn.execute(
                """
                SELECT * FROM security_ask_events
                WHERE event_id = ? AND rule_id = 'corp.rules.ask_ironbank_mock_http'
                ORDER BY id
                """,
                (event_id,),
            ).fetchall()
            assert len(ask_rows) == 1, [dict(row) for row in ask_rows]
            ask_row = dict(ask_rows[0])
            ask_id = _event_id(ask_row["ask_id"])
            assert ask_row["event_id"] == event_id
            assert ask_row["event_type"] == "http.request"
            assert ask_row["rule_name"] == "ask_ironbank_mock_http"
            assert ask_row["status"] == "pending"
            assert ask_row["resolver"] is None
            assert ask_row["reason"] is None
            assert ask_row["trace_id"] == net["trace_id"]
            ask_rule_json = json.loads(ask_row["rule_json"])
            assert ask_rule_json["rule_action"] == "ask"
            assert ask_rule_json["detection_level"] == "medium"
            ask_event_json = json.loads(ask_row["event_json"])
            assert ask_event_json["event_type"] == "http.request"
            assert ask_event_json["http"]["path"] == "/ask-target"

        uds_net_rows = _query_rows(
            client,
            session_id,
            """
            SELECT event_id, method, path, status_code, decision, matched_rule,
                   policy_action, policy_rule, request_body_preview,
                   response_body_preview, trace_id
            FROM net_events
            WHERE event_id = '%s'
            """
            % event_id,
        )
        assert len(uds_net_rows) == 1
        assert uds_net_rows[0]["decision"] == "denied"
        assert uds_net_rows[0]["policy_action"] == "ask"
        assert uds_net_rows[0]["request_body_preview"] == result["request_body"]
        assert uds_net_rows[0]["response_body_preview"] == result["body"]

        uds_ask_rows = _query_rows(
            client,
            session_id,
            """
            SELECT ask_id, event_id, event_type, rule_id, status, trace_id
            FROM security_ask_events
            WHERE ask_id = '%s'
            """
            % ask_id,
        )
        assert uds_ask_rows == [
            {
                "ask_id": ask_id,
                "event_id": event_id,
                "event_type": "http.request",
                "rule_id": "corp.rules.ask_ironbank_mock_http",
                "status": "pending",
                "trace_id": net["trace_id"],
            }
        ]

        gateway_ask_rows = gateway_client.post(
            f"/vms/{session_id}/inspect",
            {
                "sql": (
                    "SELECT ask_id, event_id, rule_id, status, trace_id "
                    f"FROM security_ask_events WHERE ask_id = '{ask_id}'"
                )
            },
            timeout=30,
        )
        assert gateway_ask_rows["rows"] == [
            [
                ask_id,
                event_id,
                "corp.rules.ask_ironbank_mock_http",
                "pending",
                net["trace_id"],
            ]
        ]

        security_latest = client.get(f"/vms/{session_id}/security/latest?limit=50", timeout=30)
        latest_row = next(
            row
            for row in security_latest
            if row["event_id"] == event_id
            and row["rule_id"] == "corp.rules.ask_ironbank_mock_http"
        )
        assert latest_row["event_type"] == "http.request"
        assert latest_row["rule_action"] == "ask"
        assert latest_row["detection_level"] == "medium"

        security_status = client.get(f"/vms/{session_id}/security/status", timeout=30)
        by_action = {row["rule_action"]: row["count"] for row in security_status["by_action"]}
        by_event_type = {
            row["event_type"]: row["count"] for row in security_status["by_event_type"]
        }
        assert by_action["ask"] >= 1
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
