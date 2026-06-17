"""Ironbank stats/detail route contract.

The desktop stats UI must be a projection of session.db and public routes, not
invented preview fields or duplicated payload renderings. This test seeds a
real session database, then reads it only through capsem-service routes.
"""

from __future__ import annotations

import json
import platform
import sqlite3
import tomllib
from pathlib import Path

import pytest

from helpers.constants import CODE_PROFILE_ID, DEFAULT_CPUS, DEFAULT_RAM_MB
from helpers.service import ServiceInstance, materialize_test_profiles


pytestmark = pytest.mark.integration

SESSION_ID = "code-stats-ledger"
TRACE_ID = "trace-stats-ledger"
HTTP_EVENT_ID = "a1b2c3d4e5f6"
MODEL_EVENT_ID = "b1c2d3e4f5a6"
MCP_EVENT_ID = "c1d2e3f4a5b6"
DNS_EVENT_ID = "d1e2f3a4b5c6"
FILE_EVENT_ID = "e1f2a3b4c5d6"
EXEC_EVENT_ID = "f1a2b3c4d5e6"
CRED_EVENT_ID = "abc123def456"
SEC_EVENT_ID = "123abc456def"
CREDENTIAL_REF = "credential:blake3:" + "1" * 64
BLAKE3_HASH = "blake3:" + "2" * 64


def _profile_contract(tmp_dir: Path) -> dict[str, object]:
    profiles_dir = materialize_test_profiles(tmp_dir)
    profile = tomllib.loads((profiles_dir / CODE_PROFILE_ID / "profile.toml").read_text())
    arch = "arm64" if platform.machine().lower() in ("arm64", "aarch64") else "x86_64"
    assets = profile["assets"]["arch"][arch]
    return {
        "revision": profile["revision"],
        "pins": {
            "kernel": {"name": assets["kernel"]["name"], "hash": assets["kernel"]["hash"]},
            "initrd": {"name": assets["initrd"]["name"], "hash": assets["initrd"]["hash"]},
            "rootfs": {"name": assets["rootfs"]["name"], "hash": assets["rootfs"]["hash"]},
        },
    }


def _write_registry(tmp_dir: Path, session_dir: Path, contract: dict[str, object]) -> None:
    (tmp_dir / "persistent_registry.json").write_text(
        json.dumps(
            {
                "vms": {
                    SESSION_ID: {
                        "name": SESSION_ID,
                        "profile_id": CODE_PROFILE_ID,
                        "profile_revision": contract["revision"],
                        "profile_payload_hash": "blake3:" + "3" * 64,
                        "asset_pins": contract["pins"],
                        "ram_mb": DEFAULT_RAM_MB,
                        "cpus": DEFAULT_CPUS,
                        "base_version": "0.0.0-ironbank",
                        "created_at": "2026-06-17T00:00:00Z",
                        "session_dir": str(session_dir),
                        "defunct": False,
                    }
                }
            },
            indent=2,
        ),
        encoding="utf-8",
    )


def _create_schema(conn: sqlite3.Connection) -> None:
    conn.executescript(
        """
        CREATE TABLE net_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            event_id TEXT NOT NULL,
            timestamp TEXT NOT NULL,
            domain TEXT NOT NULL,
            port INTEGER DEFAULT 443,
            decision TEXT NOT NULL,
            process_name TEXT,
            pid INTEGER,
            method TEXT,
            path TEXT,
            query TEXT,
            status_code INTEGER,
            bytes_sent INTEGER DEFAULT 0,
            bytes_received INTEGER DEFAULT 0,
            duration_ms INTEGER DEFAULT 0,
            matched_rule TEXT,
            request_headers TEXT,
            response_headers TEXT,
            request_body_preview TEXT,
            response_body_preview TEXT,
            conn_type TEXT DEFAULT 'https',
            policy_mode TEXT,
            policy_action TEXT,
            policy_rule TEXT,
            policy_reason TEXT,
            trace_id TEXT,
            credential_ref TEXT
        );
        CREATE TABLE model_calls (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            event_id TEXT NOT NULL,
            timestamp TEXT NOT NULL,
            provider TEXT NOT NULL,
            protocol TEXT,
            model TEXT,
            process_name TEXT,
            pid INTEGER,
            method TEXT NOT NULL,
            path TEXT NOT NULL,
            stream INTEGER DEFAULT 0,
            system_prompt_preview TEXT,
            messages_count INTEGER DEFAULT 0,
            tools_count INTEGER DEFAULT 0,
            request_bytes INTEGER DEFAULT 0,
            request_body_preview TEXT,
            message_id TEXT,
            status_code INTEGER,
            text_content TEXT,
            thinking_content TEXT,
            stop_reason TEXT,
            input_tokens INTEGER,
            output_tokens INTEGER,
            duration_ms INTEGER DEFAULT 0,
            response_bytes INTEGER DEFAULT 0,
            estimated_cost_usd REAL DEFAULT 0,
            trace_id TEXT,
            usage_details TEXT,
            credential_ref TEXT
        );
        CREATE TABLE mcp_calls (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            event_id TEXT NOT NULL,
            timestamp TEXT NOT NULL,
            server_name TEXT NOT NULL,
            method TEXT NOT NULL,
            tool_name TEXT,
            request_id TEXT,
            request_preview TEXT,
            response_preview TEXT,
            decision TEXT NOT NULL,
            duration_ms INTEGER DEFAULT 0,
            error_message TEXT,
            process_name TEXT,
            bytes_sent INTEGER DEFAULT 0,
            bytes_received INTEGER DEFAULT 0,
            policy_mode TEXT,
            policy_action TEXT,
            policy_rule TEXT,
            policy_reason TEXT,
            trace_id TEXT,
            credential_ref TEXT
        );
        CREATE TABLE event_body_blobs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            event_id TEXT NOT NULL,
            event_type TEXT NOT NULL,
            source_table TEXT NOT NULL,
            direction TEXT NOT NULL,
            content_type TEXT,
            original_bytes INTEGER NOT NULL,
            stored_bytes INTEGER NOT NULL,
            truncated INTEGER NOT NULL,
            body_hash TEXT NOT NULL,
            body BLOB NOT NULL,
            trace_id TEXT,
            created_at TEXT NOT NULL
        );
        CREATE TABLE dns_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            event_id TEXT NOT NULL,
            timestamp TEXT NOT NULL,
            qname TEXT NOT NULL,
            qtype INTEGER NOT NULL,
            qclass INTEGER NOT NULL,
            rcode INTEGER NOT NULL,
            answer_ip TEXT,
            decision TEXT NOT NULL,
            matched_rule TEXT,
            source_proto TEXT,
            process_name TEXT,
            upstream_resolver_ms INTEGER DEFAULT 0,
            trace_id TEXT,
            policy_mode TEXT,
            policy_action TEXT,
            policy_rule TEXT,
            policy_reason TEXT,
            credential_ref TEXT
        );
        CREATE TABLE fs_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            event_id TEXT NOT NULL,
            timestamp TEXT NOT NULL,
            action TEXT NOT NULL,
            path TEXT NOT NULL,
            directory TEXT,
            name TEXT,
            size INTEGER,
            trace_id TEXT,
            credential_ref TEXT
        );
        CREATE TABLE exec_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            event_id TEXT NOT NULL,
            timestamp TEXT NOT NULL,
            exec_id INTEGER NOT NULL,
            command TEXT NOT NULL,
            exit_code INTEGER,
            duration_ms INTEGER,
            stdout_preview TEXT,
            stderr_preview TEXT,
            stdout_bytes INTEGER DEFAULT 0,
            stderr_bytes INTEGER DEFAULT 0,
            source TEXT NOT NULL DEFAULT 'api',
            mcp_call_id INTEGER,
            trace_id TEXT,
            process_name TEXT,
            pid INTEGER,
            credential_ref TEXT
        );
        CREATE TABLE audit_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            event_id TEXT NOT NULL,
            timestamp TEXT NOT NULL,
            pid INTEGER NOT NULL,
            ppid INTEGER NOT NULL,
            uid INTEGER NOT NULL,
            exe TEXT NOT NULL,
            comm TEXT,
            argv TEXT NOT NULL,
            cwd TEXT,
            exit_code INTEGER,
            session_id INTEGER,
            tty TEXT,
            audit_id TEXT,
            exec_event_id INTEGER,
            parent_exe TEXT,
            trace_id TEXT,
            credential_ref TEXT
        );
        CREATE TABLE substitution_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            event_id TEXT NOT NULL,
            timestamp TEXT NOT NULL,
            material_class TEXT NOT NULL,
            source TEXT NOT NULL,
            event_type TEXT,
            algorithm TEXT NOT NULL,
            substitution_ref TEXT NOT NULL,
            outcome TEXT NOT NULL,
            provider TEXT,
            confidence REAL,
            trace_id TEXT,
            context_json TEXT
        );
        CREATE TABLE security_rule_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp_unix_ms INTEGER NOT NULL,
            event_id TEXT NOT NULL,
            event_type TEXT NOT NULL,
            rule_id TEXT NOT NULL,
            rule_action TEXT NOT NULL,
            detection_level TEXT NOT NULL DEFAULT 'none',
            rule_json TEXT NOT NULL,
            event_json TEXT NOT NULL,
            trace_id TEXT
        );
        """
    )


def _seed_session_db(db_path: Path) -> None:
    request_body = json.dumps({"prompt": "write the ledger poem", "nonce": "stats-detail"})
    full_response = json.dumps({"poem": "ironbank-" + ("x" * 70_000) + "-tail"})
    model_response = "Thought for 2s.\nCreated /root/poeme.md with a ledger poem."
    mcp_response = json.dumps({"content": [{"type": "text", "text": "created poeme.md"}]})
    rule_json = json.dumps(
        {
            "name": "stats_detail_google_detect",
            "action": "allow",
            "detection_level": "informational",
            "match": 'http.host.contains("googleapis.com")',
        },
        sort_keys=True,
    )
    event_json = json.dumps(
        {
            "event_id": SEC_EVENT_ID,
            "event_type": "http.request",
            "http": {"host": "daily-cloudcode-pa.googleapis.com", "path": "/v1internal"},
        },
        sort_keys=True,
    )

    conn = sqlite3.connect(db_path)
    try:
        _create_schema(conn)
        conn.execute(
            """
            INSERT INTO net_events (
                event_id, timestamp, domain, port, decision, method, path, query,
                status_code, bytes_sent, bytes_received, duration_ms, matched_rule,
                request_headers, response_headers, request_body_preview,
                response_body_preview, conn_type, policy_rule, trace_id, credential_ref
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                HTTP_EVENT_ID,
                "2026-06-17T20:11:18.404098Z",
                "daily-cloudcode-pa.googleapis.com",
                443,
                "allowed",
                "POST",
                "/v1internal:listExperiments",
                None,
                200,
                len(request_body),
                len(full_response),
                124,
                "profiles.rules.ai_google_http_googleapis",
                "host: daily-cloudcode-pa.googleapis.com\ncontent-type: application/json",
                "content-type: application/json",
                request_body,
                full_response,
                "https-mitm",
                "profiles.rules.ai_google_http_googleapis",
                TRACE_ID,
                CREDENTIAL_REF,
            ),
        )
        conn.executemany(
            """
            INSERT INTO event_body_blobs (
                event_id, event_type, source_table, direction, content_type,
                original_bytes, stored_bytes, truncated, body_hash, body,
                trace_id, created_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            [
                (
                    HTTP_EVENT_ID,
                    "http.request",
                    "net_events",
                    "request",
                    "application/json",
                    len(request_body.encode()),
                    len(request_body.encode()),
                    0,
                    BLAKE3_HASH,
                    request_body.encode(),
                    TRACE_ID,
                    "2026-06-17T20:11:18.404098Z",
                ),
                (
                    HTTP_EVENT_ID,
                    "http.request",
                    "net_events",
                    "response",
                    "application/json",
                    len(full_response.encode()),
                    len(full_response.encode()),
                    0,
                    BLAKE3_HASH,
                    full_response.encode(),
                    TRACE_ID,
                    "2026-06-17T20:11:18.404198Z",
                ),
                (
                    MODEL_EVENT_ID,
                    "model.call",
                    "model_calls",
                    "response",
                    "text/plain",
                    len(model_response.encode()),
                    len(model_response.encode()),
                    0,
                    BLAKE3_HASH,
                    model_response.encode(),
                    TRACE_ID,
                    "2026-06-17T20:11:19Z",
                ),
                (
                    MCP_EVENT_ID,
                    "mcp.tool_call",
                    "mcp_calls",
                    "response",
                    "application/json",
                    len(mcp_response.encode()),
                    len(mcp_response.encode()),
                    0,
                    BLAKE3_HASH,
                    mcp_response.encode(),
                    TRACE_ID,
                    "2026-06-17T20:11:20Z",
                ),
            ],
        )
        conn.execute(
            """
            INSERT INTO model_calls (
                event_id, timestamp, provider, protocol, model, process_name, pid,
                method, path, stream, messages_count, tools_count, request_bytes,
                request_body_preview, message_id, status_code, text_content,
                thinking_content, stop_reason, input_tokens, output_tokens,
                duration_ms, response_bytes, estimated_cost_usd, trace_id, credential_ref
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                MODEL_EVENT_ID,
                "2026-06-17T20:11:19Z",
                "google",
                "google",
                "gemini-3.5-flash",
                "agy",
                215,
                "POST",
                "/v1beta/models/gemini-3.5-flash:streamGenerateContent",
                1,
                2,
                1,
                len(request_body),
                request_body,
                "msg-stats-detail",
                200,
                "Created poeme.md.",
                "Clarifying file destination.",
                "stop",
                542,
                27,
                931,
                len(model_response),
                0.00042,
                TRACE_ID,
                CREDENTIAL_REF,
            ),
        )
        conn.execute(
            """
            INSERT INTO mcp_calls (
                event_id, timestamp, server_name, method, tool_name, request_id,
                request_preview, response_preview, decision, duration_ms,
                process_name, bytes_sent, bytes_received, policy_rule, trace_id,
                credential_ref
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                MCP_EVENT_ID,
                "2026-06-17T20:11:20Z",
                "builtin",
                "tools/call",
                "create_file",
                "mcp-req-1",
                json.dumps({"name": "create_file", "arguments": {"path": "/root/poeme.md"}}),
                mcp_response,
                "allowed",
                12,
                "agy",
                88,
                len(mcp_response),
                "profiles.rules.default_mcp",
                TRACE_ID,
                None,
            ),
        )
        conn.execute(
            """
            INSERT INTO dns_events (
                event_id, timestamp, qname, qtype, qclass, rcode, answer_ip,
                decision, matched_rule, source_proto, process_name,
                upstream_resolver_ms, trace_id, policy_rule, credential_ref
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                DNS_EVENT_ID,
                "2026-06-17T20:11:17Z",
                "daily-cloudcode-pa.googleapis.com",
                1,
                1,
                0,
                "142.250.72.10",
                "allowed",
                "profiles.rules.default_dns",
                "udp",
                "agy",
                29,
                TRACE_ID,
                "profiles.rules.default_dns",
                None,
            ),
        )
        conn.execute(
            """
            INSERT INTO fs_events (
                event_id, timestamp, action, path, directory, name, size,
                trace_id, credential_ref
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                FILE_EVENT_ID,
                "2026-06-17T20:11:21Z",
                "created",
                "/root/poeme.md",
                "/root",
                "poeme.md",
                96,
                TRACE_ID,
                None,
            ),
        )
        conn.execute(
            """
            INSERT INTO exec_events (
                event_id, timestamp, exec_id, command, exit_code, duration_ms,
                stdout_preview, stderr_preview, stdout_bytes, stderr_bytes,
                source, trace_id, process_name, pid, credential_ref
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                EXEC_EVENT_ID,
                "2026-06-17T20:11:16Z",
                7,
                "agy --allow-dangerous-permissions",
                0,
                15,
                "Antigravity CLI 1.0.8",
                "",
                23,
                0,
                "api",
                TRACE_ID,
                "agy",
                215,
                None,
            ),
        )
        conn.execute(
            """
            INSERT INTO audit_events (
                event_id, timestamp, pid, ppid, uid, exe, comm, argv, cwd,
                exit_code, session_id, tty, audit_id, exec_event_id, parent_exe,
                trace_id, credential_ref
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "fedcba654321",
                "2026-06-17T20:11:16Z",
                215,
                1,
                0,
                "/usr/local/bin/agy",
                "agy",
                json.dumps(["agy", "--allow-dangerous-permissions"]),
                "/root",
                None,
                1,
                "pts/0",
                "audit-1",
                7,
                "/usr/bin/bash",
                TRACE_ID,
                None,
            ),
        )
        conn.executemany(
            """
            INSERT INTO substitution_events (
                event_id, timestamp, material_class, source, event_type,
                algorithm, substitution_ref, outcome, provider, trace_id,
                context_json
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            [
                (
                    CRED_EVENT_ID,
                    "2026-06-17T20:11:15Z",
                    "credential",
                    "http.body.response.$.access_token",
                    "http.request",
                    "blake3",
                    CREDENTIAL_REF,
                    "captured",
                    "google",
                    TRACE_ID,
                    json.dumps({"domain": "oauth2.googleapis.com"}),
                ),
                (
                    "abc123def457",
                    "2026-06-17T20:11:16Z",
                    "credential",
                    "http.header.authorization",
                    "http.request",
                    "blake3",
                    CREDENTIAL_REF,
                    "injected",
                    "google",
                    TRACE_ID,
                    json.dumps({"domain": "daily-cloudcode-pa.googleapis.com"}),
                ),
            ],
        )
        conn.executemany(
            """
            INSERT INTO security_rule_events (
                timestamp_unix_ms, event_id, event_type, rule_id, rule_action,
                detection_level, rule_json, event_json, trace_id
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            [
                (
                    1_789_000_223_456,
                    SEC_EVENT_ID,
                    "http.request",
                    "profiles.rules.ai_google_http_googleapis",
                    "allow",
                    "informational",
                    rule_json,
                    event_json,
                    TRACE_ID,
                ),
                (
                    1_789_000_223_457,
                    "223abc456def",
                    "mcp.tool_call",
                    "profiles.rules.default_mcp",
                    "ask",
                    "none",
                    json.dumps({"name": "default_mcp", "action": "ask"}, sort_keys=True),
                    json.dumps({"event_type": "mcp.tool_call", "mcp": {"name": "create_file"}}),
                    TRACE_ID,
                ),
            ],
        )
        conn.commit()
    finally:
        conn.close()


def _query(client, sql: str) -> list[dict[str, object]]:
    payload = client.post(f"/vms/{SESSION_ID}/inspect", {"sql": sql}, timeout=30)
    assert set(payload) == {"columns", "rows"}, payload
    return [dict(zip(payload["columns"], row, strict=True)) for row in payload["rows"]]


def test_stats_detail_routes_project_session_db_without_preview_theater() -> None:
    service = ServiceInstance()
    try:
        session_dir = service.tmp_dir / "persistent" / SESSION_ID
        session_dir.mkdir(parents=True, exist_ok=True)
        contract = _profile_contract(service.tmp_dir)
        _seed_session_db(session_dir / "session.db")
        _write_registry(service.tmp_dir, session_dir, contract)

        service.start()
        client = service.client()

        http_rows = _query(
            client,
            """
            SELECT event_id, timestamp, domain, port, method, path, query,
                   status_code, decision, duration_ms, bytes_sent, bytes_received,
                   matched_rule, policy_rule, trace_id, credential_ref,
                   request_headers, response_headers
            FROM net_events
            ORDER BY id DESC
            LIMIT 200
            """,
        )
        assert len(http_rows) == 1
        http = http_rows[0]
        assert http["event_id"] == HTTP_EVENT_ID
        assert http["domain"] == "daily-cloudcode-pa.googleapis.com"
        assert http["status_code"] == 200
        assert http["credential_ref"] == CREDENTIAL_REF
        assert "request_body_preview" not in http
        assert "response_body_preview" not in http

        body_rows = _query(
            client,
            f"""
            SELECT direction, content_type, original_bytes, stored_bytes,
                   truncated, body_hash, CAST(body AS TEXT) AS body
            FROM event_body_blobs
            WHERE event_id = '{HTTP_EVENT_ID}'
            ORDER BY direction
            """,
        )
        bodies = {row["direction"]: row for row in body_rows}
        assert set(bodies) == {"request", "response"}
        assert json.loads(bodies["request"]["body"]) == {
            "prompt": "write the ledger poem",
            "nonce": "stats-detail",
        }
        response_body = bodies["response"]["body"]
        assert isinstance(response_body, str)
        assert response_body.endswith("-tail\"}")
        assert len(response_body) > 65_536
        assert bodies["response"]["original_bytes"] == len(response_body.encode())
        assert bodies["response"]["stored_bytes"] == len(response_body.encode())
        assert bodies["response"]["truncated"] == 0
        assert str(bodies["response"]["body_hash"]).startswith("blake3:")

        model_rows = _query(
            client,
            """
            SELECT event_id, provider, protocol, model, method, path, stream,
                   input_tokens, output_tokens, thinking_content, text_content,
                   trace_id, credential_ref
            FROM model_calls
            ORDER BY id DESC
            """,
        )
        assert model_rows == [
            {
                "event_id": MODEL_EVENT_ID,
                "provider": "google",
                "protocol": "google",
                "model": "gemini-3.5-flash",
                "method": "POST",
                "path": "/v1beta/models/gemini-3.5-flash:streamGenerateContent",
                "stream": 1,
                "input_tokens": 542,
                "output_tokens": 27,
                "thinking_content": "Clarifying file destination.",
                "text_content": "Created poeme.md.",
                "trace_id": TRACE_ID,
                "credential_ref": CREDENTIAL_REF,
            }
        ]

        mcp_rows = _query(
            client,
            """
            SELECT event_id, server_name, method, tool_name, request_id,
                   decision, duration_ms, bytes_sent, bytes_received,
                   policy_rule, trace_id, credential_ref, error_message
            FROM mcp_calls
            ORDER BY id DESC
            """,
        )
        assert mcp_rows[0]["server_name"] == "builtin"
        assert mcp_rows[0]["method"] == "tools/call"
        assert mcp_rows[0]["tool_name"] == "create_file"
        assert mcp_rows[0]["policy_rule"] == "profiles.rules.default_mcp"
        assert mcp_rows[0]["error_message"] is None

        dns_rows = _query(
            client,
            """
            SELECT event_id, qname, qtype, qclass, rcode, answer_ip,
                   decision, source_proto, process_name, upstream_resolver_ms,
                   trace_id, credential_ref
            FROM dns_events
            ORDER BY id DESC
            """,
        )
        assert dns_rows[0]["qname"] == "daily-cloudcode-pa.googleapis.com"
        assert dns_rows[0]["qtype"] == 1
        assert dns_rows[0]["answer_ip"] == "142.250.72.10"

        file_rows = _query(
            client,
            """
            SELECT event_id, action, path, directory, name, size, trace_id, credential_ref
            FROM fs_events
            ORDER BY id DESC
            """,
        )
        assert file_rows[0]["action"] == "created"
        assert file_rows[0]["path"] == "/root/poeme.md"
        assert file_rows[0]["name"] == "poeme.md"

        exec_rows = _query(
            client,
            """
            SELECT event_id, exec_id, command, exit_code, duration_ms,
                   stdout_bytes, stderr_bytes, source, process_name, pid,
                   trace_id, credential_ref
            FROM exec_events
            ORDER BY id DESC
            """,
        )
        assert exec_rows[0]["command"] == "agy --allow-dangerous-permissions"
        assert exec_rows[0]["source"] == "api"
        assert exec_rows[0]["process_name"] == "agy"

        audit_rows = _query(
            client,
            """
            SELECT event_id, pid, ppid, uid, exe, comm, argv, cwd,
                   exit_code, session_id, tty, audit_id, exec_event_id,
                   parent_exe, trace_id, credential_ref
            FROM audit_events
            ORDER BY id DESC
            """,
        )
        assert audit_rows[0]["exe"] == "/usr/local/bin/agy"
        assert json.loads(audit_rows[0]["argv"]) == ["agy", "--allow-dangerous-permissions"]
        assert audit_rows[0]["exec_event_id"] == 7

        credential_rows = _query(
            client,
            """
            SELECT event_id, timestamp, material_class, source, event_type,
                   event_type AS origin, outcome AS verb, provider,
                   trace_id, context_json
            FROM substitution_events
            ORDER BY id ASC
            """,
        )
        assert [row["verb"] for row in credential_rows] == ["captured", "injected"]
        assert all("substitution_ref" not in row for row in credential_rows)
        assert all("confidence" not in row for row in credential_rows)
        assert credential_rows[0]["provider"] == "google"
        assert json.loads(credential_rows[0]["context_json"]) == {
            "domain": "oauth2.googleapis.com"
        }

        latest = client.get(f"/vms/{SESSION_ID}/security/latest?limit=10", timeout=30)
        assert [row["event_id"] for row in latest] == ["223abc456def", SEC_EVENT_ID]
        assert latest[1]["rule_id"] == "profiles.rules.ai_google_http_googleapis"
        assert latest[1]["rule_action"] == "allow"
        assert latest[1]["detection_level"] == "informational"
        assert json.loads(latest[1]["event_json"])["http"]["host"] == (
            "daily-cloudcode-pa.googleapis.com"
        )

        security = client.get(f"/vms/{SESSION_ID}/security/status", timeout=30)
        assert security["total"] == 2
        assert {row["rule_action"]: row["count"] for row in security["by_action"]} == {
            "allow": 1,
            "ask": 1,
        }
        assert {row["detection_level"]: row["count"] for row in security["by_level"]} == {
            "informational": 1,
            "none": 1,
        }
        assert {row["event_type"]: row["count"] for row in security["by_event_type"]} == {
            "http.request": 1,
            "mcp.tool_call": 1,
        }

        detection_latest = client.get(f"/vms/{SESSION_ID}/detection/latest?limit=10", timeout=30)
        enforcement_latest = client.get(
            f"/vms/{SESSION_ID}/enforcement/latest?limit=10",
            timeout=30,
        )
        assert detection_latest == latest
        assert enforcement_latest == latest
    finally:
        service.stop()
