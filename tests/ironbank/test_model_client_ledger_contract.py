"""Ironbank model client ledger contract tests.

Each test owns one client surface and one deterministic tool-use exchange.
The shared assertion reconciles the client result, upstream transcript,
session DB, security ledger, files, and logs.
"""

from __future__ import annotations

from contextlib import closing
from dataclasses import dataclass
import json
import os
from pathlib import Path
import textwrap
import time
import uuid

import pytest

from helpers.constants import CODE_PROFILE_ID, DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.mock_server import MOCK_SERVER_BINARY, start_mock_server, stop_process
from helpers.service import ServiceInstance, wait_exec_ready, vm_name
from ironbank.model_client_assertions import assert_one_model_client
from ironbank.model_client_scripts import (
    agy_cli_script,
    claude_api_script,
    claude_sdk_script,
    codex_cli_script,
    openai_responses_api_script,
    openai_two_tool_calls_script,
)

pytestmark = pytest.mark.integration

PROJECT_ROOT = Path(__file__).resolve().parents[2]
ASSETS_DIR = PROJECT_ROOT / "assets"
PROFILES_DIR = PROJECT_ROOT / "target" / "config" / "profiles"


def _eventually(query, predicate, *, timeout_s: float = 10.0, interval_s: float = 0.25):
    deadline = time.monotonic() + timeout_s
    last = None
    while time.monotonic() < deadline:
        last = query()
        if predicate(last):
            return last
        time.sleep(interval_s)
    last = query()
    assert predicate(last), last
    return last


def _credential_ref(value: object) -> str:
    import re

    assert isinstance(value, str)
    assert re.fullmatch(r"credential:blake3:[0-9a-f]{64}", value), value
    return value


def _assert_raw_absent_from_db(conn, raw_secret: str) -> None:
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
                assert raw_secret not in str(value), f"raw secret leaked in {table}.{column}"


@dataclass
class ModelClientEnv:
    service: ServiceInstance
    client: object
    session_id: str
    mock_base_url: str
    upstream_transcript_path: Path

    @property
    def db_path(self) -> Path:
        return self.service.tmp_dir / "sessions" / self.session_id / "session.db"

    @property
    def log_paths(self) -> tuple[Path, ...]:
        session_dir = self.service.tmp_dir / "sessions" / self.session_id
        return (
            self.service.tmp_dir / "service.log",
            self.service.tmp_dir / "service.stderr.log",
            session_dir / "process.log",
            session_dir / "serial.log",
        )

    def run_python(self, script: str, *, timeout_secs: int = 240) -> dict:
        script_name = f"ironbank-client-{uuid.uuid4().hex[:8]}.py"
        payload = script.encode()
        upload = self.client.post_bytes(
            f"/vms/{self.session_id}/files/content?path={script_name}",
            payload,
            timeout=30,
        )
        assert upload is not None
        assert upload["success"] is True
        assert upload["size"] == len(payload)
        exec_resp = self.client.post(
            f"/vms/{self.session_id}/exec",
            {"command": f"python3 /root/{script_name}", "timeout_secs": timeout_secs},
            timeout=timeout_secs + 30,
        )
        assert exec_resp is not None
        assert exec_resp["exit_code"] == 0, exec_resp
        stdout = exec_resp.get("stdout") or ""
        stderr = exec_resp.get("stderr") or ""
        line = next(
            (line for line in stdout.splitlines() if line.startswith("IRONBANK_CLIENT_RESULT=")),
            None,
        )
        assert line is not None, stdout + stderr
        return json.loads(line.split("=", 1)[1])


@pytest.fixture
def model_client_env():
    assert MOCK_SERVER_BINARY.exists(), f"{MOCK_SERVER_BINARY} missing"
    assert ASSETS_DIR.exists(), f"{ASSETS_DIR} missing; build VM assets before Ironbank"
    assert PROFILES_DIR.exists(), f"{PROFILES_DIR} missing; materialize profile config"

    service = ServiceInstance()
    client = None
    mock_proc = None
    old_corp_config = os.environ.get("CAPSEM_CORP_CONFIG")
    session_id = vm_name("ironbank-model")
    try:
        mock_proc, ready = start_mock_server(
            request_log=service.tmp_dir / "upstream-transcript.jsonl"
        )
        corp_path = service.tmp_dir / "corp.toml"
        corp_path.write_text(
            textwrap.dedent(
                f"""
                refresh_policy = "24h"

                [network.dns]
                upstreams = [{json.dumps(ready["dns_udp_addr"])}]

                [settings."security.web.http_upstream_ports"]
                value = [80, 3713, 8080]
                modified = "2026-06-14T00:00:00Z"

                [corp.rules.allow_ironbank_mock_model_server]
                name = "allow_ironbank_mock_model_server"
                action = "allow"
                priority = -100
                detection_level = "informational"
                reason = "Allow the hermetic Ironbank model fixture while preserving local-network ask defaults."
                match = 'http.host == "127.0.0.1" && tcp.port == "3713" && (http.path == "/v1/responses" || http.path == "/v1/messages" || http.path == "/api/chat")'
                """
            ).strip()
            + "\n",
            encoding="utf-8",
        )
        os.environ["CAPSEM_CORP_CONFIG"] = str(corp_path)
        service.start()
        client = service.client()
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
        active_profile = service.tmp_dir / "sessions" / session_id / "vm" / "active_profile.toml"
        assert active_profile.exists(), f"active profile missing at {active_profile}"
        active_profile_text = active_profile.read_text(encoding="utf-8")
        assert ready["dns_udp_addr"] in active_profile_text
        assert "runtime-overlay.toml" not in active_profile_text
        assert wait_exec_ready(client, session_id, timeout=EXEC_READY_TIMEOUT)
        yield ModelClientEnv(
            service=service,
            client=client,
            session_id=session_id,
            mock_base_url=ready["base_url"],
            upstream_transcript_path=Path(ready["request_log"]),
        )
    finally:
        stop_process(mock_proc)
        if client is not None:
            try:
                client.delete(f"/vms/{session_id}/delete", timeout=60)
            except Exception:
                pass
        service.stop()
        if old_corp_config is None:
            os.environ.pop("CAPSEM_CORP_CONFIG", None)
        else:
            os.environ["CAPSEM_CORP_CONFIG"] = old_corp_config


def test_openai_responses_api_ledger_contract(model_client_env: ModelClientEnv):
    assert_one_model_client(model_client_env, openai_responses_api_script(model_client_env.mock_base_url))


def test_openai_two_tool_calls_have_exact_item_cardinality(
    model_client_env: ModelClientEnv,
):
    result = model_client_env.run_python(openai_two_tool_calls_script(model_client_env.mock_base_url))
    assert len(result["results"]) == 2, result
    assert all(item["file_matches"] for item in result["results"]), result
    assert len({item["call_id"] for item in result["results"]}) == 2, result
    assert len({item["filename"] for item in result["results"]}) == 2, result
    raw_secret = "sk-" + result["credential_nonce"]

    import sqlite3

    with closing(sqlite3.connect(f"file:{model_client_env.db_path}?mode=ro", uri=True)) as conn:
        conn.row_factory = sqlite3.Row
        tables = {
            row[0]
            for row in conn.execute(
                "SELECT name FROM sqlite_master WHERE type = 'table'"
            ).fetchall()
        }
        assert "model_items" in tables, (
            "RED: OpenAI two-tool-call ledger needs first-class model_items rows "
            "with per-trace exact cardinality: one request, one reasoning, "
            "one response, one tool_call, one tool_response, and one created file"
        )
        model_calls = conn.execute(
            """
            SELECT *
            FROM model_calls
            WHERE provider = 'openai'
              AND path = '/v1/responses'
              AND model = 'gemma4:latest'
            ORDER BY id
            """
        ).fetchall()
        assert len(model_calls) == 4, [dict(row) for row in model_calls]
        assert {row["method"] for row in model_calls} == {"POST"}
        assert {row["status_code"] for row in model_calls} == {200}
        assert all(row["request_bytes"] > 0 for row in model_calls)
        assert all(row["response_bytes"] > 0 for row in model_calls)

        item_rows = conn.execute(
            """
            SELECT *
            FROM model_items
            WHERE provider = 'openai'
              AND path = '/v1/responses'
              AND model = 'gemma4:latest'
            ORDER BY id
            """
        ).fetchall()
        by_trace: dict[str, list[sqlite3.Row]] = {}
        for row in item_rows:
            by_trace.setdefault(row["trace_id"], []).append(row)
        assert len(by_trace) == 2, [dict(row) for row in item_rows]
        assert len(item_rows) == 10, [dict(row) for row in item_rows]
        assert all(row["provider"] == "openai" for row in item_rows)
        assert all(row["path"] == "/v1/responses" for row in item_rows)
        assert all(row["model"] == "gemma4:latest" for row in item_rows)
        assert all(
            isinstance(row["content_hash"], str)
            and len(row["content_hash"]) == 71
            and row["content_hash"].startswith("blake3:")
            for row in item_rows
        )

        tool_calls = conn.execute(
            "SELECT * FROM tool_calls WHERE tool_name = 'exec_command' ORDER BY id"
        ).fetchall()
        tool_responses = conn.execute("SELECT * FROM tool_responses ORDER BY id").fetchall()
        expected_filenames = {item["filename"] for item in result["results"]}
        file_rows = _eventually(
            lambda: conn.execute(
                """
                SELECT *
                FROM fs_events
                WHERE action = 'created'
                ORDER BY id
                """
            ).fetchall(),
            lambda rows: expected_filenames
            <= {row["name"] for row in rows if row["name"] is not None},
            timeout_s=15,
        )
        net_rows = conn.execute(
            """
            SELECT *
            FROM net_events
            WHERE domain = '127.0.0.1'
              AND path = '/v1/responses'
            ORDER BY id
            """
        ).fetchall()
        assert len(net_rows) == 4, [dict(row) for row in net_rows]
        assert all(row["method"] == "POST" for row in net_rows)
        assert all(row["status_code"] == 200 for row in net_rows)
        assert all(row["decision"] == "allowed" for row in net_rows)
        assert all(row["bytes_sent"] > 0 for row in net_rows)
        assert all(row["bytes_received"] > 0 for row in net_rows)
        credential_refs = {_credential_ref(row["credential_ref"]) for row in net_rows}
        assert len(credential_refs) == 1, [dict(row) for row in net_rows]
        credential_ref = next(iter(credential_refs))
        assert {row["credential_ref"] for row in model_calls} == {credential_ref}, [
            dict(row) for row in model_calls
        ]
        assert {row["credential_ref"] for row in tool_calls} == {credential_ref}, [
            dict(row) for row in tool_calls
        ]
        assert {row["credential_ref"] for row in tool_responses} == {credential_ref}, [
            dict(row) for row in tool_responses
        ]
        substitution_rows = conn.execute(
            """
            SELECT *
            FROM substitution_events
            WHERE substitution_ref = ?
            ORDER BY id
            """,
            (credential_ref,),
        ).fetchall()
        assert substitution_rows, credential_ref
        assert {"captured", "brokered"} <= {row["outcome"] for row in substitution_rows}, [
            dict(row) for row in substitution_rows
        ]
        assert all(row["provider"] == "openai" for row in substitution_rows)
        assert all(row["algorithm"] == "blake3" for row in substitution_rows)
        assert all(row["material_class"] == "credential" for row in substitution_rows)
        assert "http.header.authorization" in {
            row["source"] for row in substitution_rows if row["outcome"] == "captured"
        }
        _assert_raw_absent_from_db(conn, raw_secret)

        dns_rows = conn.execute(
            """
            SELECT *
            FROM dns_events
            WHERE qname = ?
            ORDER BY id
            """,
            (result["dns_qname"],),
        ).fetchall()
        assert len(dns_rows) == 1, [dict(row) for row in dns_rows]
        dns = dns_rows[0]
        assert dns["qtype"] == 1, dict(dns)
        assert dns["qclass"] == 1, dict(dns)
        assert dns["rcode"] == 0, dict(dns)
        assert dns["decision"] == "allowed", dict(dns)
        assert dns["answer_ip"] == result["dns_ip"] == "127.0.0.1", dict(dns)
        assert dns["source_proto"] in {"udp", "tcp"}, dict(dns)

        file_event_ids = []
        for expected in result["results"]:
            trace_matches = [
                trace_id
                for trace_id, rows in by_trace.items()
                if any(expected["input"] in (row["content"] or "") for row in rows)
                or any(expected["output"] in (row["content"] or "") for row in rows)
            ]
            assert len(trace_matches) == 1, {
                "expected": expected,
                "model_items": [dict(row) for row in item_rows],
            }
            trace_id = trace_matches[0]
            rows = by_trace[trace_id]
            trace_model_calls = [row for row in model_calls if row["trace_id"] == trace_id]
            assert len(trace_model_calls) == 2, [dict(row) for row in model_calls]
            trace_net_rows = [row for row in net_rows if row["trace_id"] == trace_id]
            assert len(trace_net_rows) == 2, [dict(row) for row in net_rows]

            assert sum(row["kind"] == "request" for row in rows) == 1
            assert sum(row["kind"] == "reasoning" for row in rows) == 1
            assert sum(row["kind"] == "response" for row in rows) == 1
            assert sum(row["kind"] == "tool_call" for row in rows) == 1
            assert sum(row["kind"] == "tool_response" for row in rows) == 1
            request_row = next(row for row in rows if row["kind"] == "request")
            reasoning_row = next(row for row in rows if row["kind"] == "reasoning")
            response_row = next(row for row in rows if row["kind"] == "response")
            tool_call_row = next(row for row in rows if row["kind"] == "tool_call")
            tool_response_row = next(row for row in rows if row["kind"] == "tool_response")

            assert expected["input"] in (request_row["content"] or "")
            assert expected["target"] in (request_row["content"] or "")
            assert '"tools"' in (request_row["content"] or "")
            assert "exec_command" in (request_row["content"] or "")
            assert reasoning_row["content"] == expected["reasoning"]
            assert response_row["content"] == expected["output"]
            assert tool_call_row["call_id"] == expected["call_id"]
            assert tool_call_row["tool_name"] == expected["tool_call_name"]
            assert json.loads(tool_call_row["arguments"]) == expected["call_args"]
            assert expected["target"] in (tool_call_row["content"] or "")
            assert expected["nonce"] in (tool_call_row["content"] or "")
            assert tool_response_row["call_id"] == expected["call_id"]
            assert tool_response_row["content"] == expected["call_response"]

            trace_tool_calls = [row for row in tool_calls if row["trace_id"] == trace_id]
            assert len(trace_tool_calls) == 1, [dict(row) for row in tool_calls]
            assert trace_tool_calls[0]["call_id"] == expected["call_id"]
            assert json.loads(trace_tool_calls[0]["arguments"]) == expected["call_args"]
            trace_tool_responses = [
                row for row in tool_responses if row["trace_id"] == trace_id
            ]
            assert len(trace_tool_responses) == 1, [dict(row) for row in tool_responses]
            assert trace_tool_responses[0]["call_id"] == expected["call_id"]
            assert expected["call_response"] in (
                trace_tool_responses[0]["content_preview"] or ""
            )
            created = [
                row
                for row in file_rows
                if row["trace_id"] == trace_id and row["name"] == expected["filename"]
            ]
            assert len(created) == 1, [dict(row) for row in file_rows]
            assert created[0]["size"] == len((expected["nonce"] + "\n").encode())
            assert created[0]["directory"] == ".", dict(created[0])
            assert created[0]["credential_ref"] == credential_ref, dict(created[0])
            file_event_ids.append(created[0]["event_id"])

        event_ids = [row["event_id"] for row in [*model_calls, *net_rows, dns]]
        event_ids.extend(file_event_ids)
        placeholders = ",".join("?" for _ in event_ids)
        rule_rows = conn.execute(
            f"""
            SELECT *
            FROM security_rule_events
            WHERE event_id IN ({placeholders})
            ORDER BY id
            """,
            event_ids,
        ).fetchall()
        assert rule_rows, event_ids
        covered = {row["event_id"] for row in rule_rows}
        assert set(event_ids) <= covered, {
            "missing": sorted(set(event_ids) - covered),
            "rows": [dict(row) for row in rule_rows],
        }
        assert all(
            row["rule_action"]
            in {"allow", "ask", "block", "preprocess", "rewrite", "postprocess"}
            for row in rule_rows
        )
        assert all(
            row["detection_level"]
            in {"none", "informational", "low", "medium", "high", "critical"}
            for row in rule_rows
        )
        assert all(json.loads(row["event_json"]) for row in rule_rows)
        assert all(json.loads(row["rule_json"]) for row in rule_rows)
    for log_path in model_client_env.log_paths:
        if log_path.exists():
            assert raw_secret not in log_path.read_text(
                encoding="utf-8", errors="replace"
            ), f"raw secret leaked in {log_path}"


def test_codex_cli_ledger_contract(model_client_env: ModelClientEnv):
    assert_one_model_client(
        model_client_env,
        codex_cli_script(model_client_env.mock_base_url),
    )


def test_claude_http_api_ledger_contract(model_client_env: ModelClientEnv):
    assert_one_model_client(
        model_client_env,
        claude_api_script(model_client_env.mock_base_url),
    )


def test_claude_sdk_ledger_contract(model_client_env: ModelClientEnv):
    assert_one_model_client(
        model_client_env,
        claude_sdk_script(model_client_env.mock_base_url),
    )


def test_agy_cli_ledger_contract(model_client_env: ModelClientEnv):
    assert_one_model_client(model_client_env, agy_cli_script(model_client_env.mock_base_url))
