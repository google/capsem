"""Ironbank black-box model SDK ledger tests."""

from __future__ import annotations

import json
import re
import sqlite3
import textwrap
import time
import uuid
from pathlib import Path

import pytest

from helpers.constants import CODE_PROFILE_ID, DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.mock_server import MOCK_SERVER_BINARY, start_mock_server, stop_process
from helpers.service import ServiceInstance, wait_exec_ready, vm_name

pytestmark = pytest.mark.integration

PROJECT_ROOT = Path(__file__).resolve().parents[2]
ASSETS_DIR = PROJECT_ROOT / "assets"
PROFILES_DIR = PROJECT_ROOT / "target" / "config" / "profiles"

RAW_SDK_SECRET = "capsem_test_sdk_api_key_repeat_0123456789abcdef"
EXPECTED_POEM = "Capsem ironbank poem\nledgers count the sparks\nno secret crosses raw"
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


def _connect_session_db(service: ServiceInstance, session_id: str) -> sqlite3.Connection:
    db_path = service.tmp_dir / "sessions" / session_id / "session.db"
    assert db_path.exists(), f"session.db missing at {db_path}"
    conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    conn.row_factory = sqlite3.Row
    return conn


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


def _table_columns(conn: sqlite3.Connection, table: str) -> set[str]:
    return {row[1] for row in conn.execute(f"PRAGMA table_info({table})").fetchall()}


def _assert_event_id(value: object) -> None:
    assert isinstance(value, str)
    assert re.fullmatch(r"[0-9a-f]{12}", value), value


def _assert_credential_ref(value: object) -> None:
    assert isinstance(value, str)
    assert re.fullmatch(r"credential:blake3:[0-9a-f]{64}", value), value


def _assert_raw_secret_not_in_db(conn: sqlite3.Connection) -> None:
    table_names = [
        row[0]
        for row in conn.execute(
            "SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name"
        ).fetchall()
    ]
    for table in table_names:
        columns = conn.execute(f"PRAGMA table_info({table})").fetchall()
        text_columns = [row[1] for row in columns if str(row[2]).upper() in {"TEXT", ""}]
        if not text_columns:
            continue
        selected = ", ".join(f'"{column}"' for column in text_columns)
        for row in conn.execute(f'SELECT {selected} FROM "{table}"').fetchall():
            for column, value in zip(text_columns, row, strict=True):
                assert RAW_SDK_SECRET not in str(value), f"raw SDK secret leaked in {table}.{column}"


def _sdk_probe_script(base_url: str) -> str:
    payload = {
        "base_url": f"{base_url.rstrip('/')}/v1",
        "api_key_parts": ["capsem_test_sdk_api_key_", "repeat_0123456789abcdef"],
        "model": "gemma4:latest",
        "poem_path": "/root/poem.md",
    }
    return textwrap.dedent(
        f"""
        import json
        from pathlib import Path

        from openai import OpenAI

        cfg = json.loads({json.dumps(json.dumps(payload))})
        client = OpenAI(base_url=cfg["base_url"], api_key="".join(cfg["api_key_parts"]))

        first = client.chat.completions.create(
            model=cfg["model"],
            messages=[
                {{"role": "system", "content": "You are a deterministic Capsem fixture."}},
                {{"role": "user", "content": "Write the Capsem ironbank poem."}},
            ],
            tools=[
                {{
                    "type": "function",
                    "function": {{
                        "name": "fixture_lookup",
                        "description": "Lookup deterministic fixture data.",
                        "parameters": {{
                            "type": "object",
                            "properties": {{"query": {{"type": "string"}}}},
                            "required": ["query"],
                        }},
                    }},
                }}
            ],
        )
        second = client.chat.completions.create(
            model=cfg["model"],
            messages=[{{"role": "user", "content": "Repeat the Capsem ironbank poem."}}],
        )

        first_message = first.choices[0].message
        second_message = second.choices[0].message
        tool_calls = first_message.tool_calls or []
        poem = first_message.content or second_message.content or ""
        Path(cfg["poem_path"]).write_text(poem + "\\n", encoding="utf-8")

        result = {{
            "first_model": first.model,
            "second_model": second.model,
            "first_content": poem,
            "second_content": second_message.content,
            "first_tool_count": len(tool_calls),
            "first_tool_name": tool_calls[0].function.name if tool_calls else None,
            "first_tool_arguments": tool_calls[0].function.arguments if tool_calls else None,
            "usage_total": (first.usage.total_tokens if first.usage else 0)
                + (second.usage.total_tokens if second.usage else 0),
            "poem_path": cfg["poem_path"],
        }}
        print("IRONBANK_SDK_RESULT=" + json.dumps(result, sort_keys=True))
        """
    ).strip()


def _broker_replay_script(base_url: str, credential_ref: str) -> str:
    payload = {
        "base_url": f"{base_url.rstrip('/')}/v1",
        "echo_url": f"{base_url.rstrip('/')}/echo",
        "token_url": f"{base_url.rstrip('/')}/oauth/token",
        "credential_response_url": f"{base_url.rstrip('/')}/credential/response",
        "credential_ref": credential_ref,
        "model": "gemma4:latest",
    }
    return textwrap.dedent(
        f"""
        import json
        import urllib.parse
        import urllib.request

        from openai import OpenAI

        cfg = json.loads({json.dumps(json.dumps(payload))})

        echo_req = urllib.request.Request(
            cfg["echo_url"],
            data=b"broker replay",
            headers={{
                "Authorization": "Bearer " + cfg["credential_ref"],
                "Content-Type": "text/plain",
            }},
            method="POST",
        )
        with urllib.request.urlopen(echo_req, timeout=30) as response:
            echo = json.loads(response.read().decode("utf-8"))

        query_echo_req = urllib.request.Request(
            cfg["echo_url"] + "?access_token=" + urllib.parse.quote(cfg["credential_ref"], safe=""),
            data=b"broker query replay",
            headers={{"Content-Type": "text/plain"}},
            method="POST",
        )
        with urllib.request.urlopen(query_echo_req, timeout=30) as response:
            query_echo = json.loads(response.read().decode("utf-8"))

        json_token_req = urllib.request.Request(
            cfg["token_url"],
            data=json.dumps({{"access_token": "capsem_test_oauth_access_json_0123456789abcdef"}}).encode("utf-8"),
            headers={{"Content-Type": "application/json"}},
            method="POST",
        )
        with urllib.request.urlopen(json_token_req, timeout=30) as response:
            json_token = json.loads(response.read().decode("utf-8"))

        form_token_req = urllib.request.Request(
            cfg["token_url"],
            data=urllib.parse.urlencode({{"code": "capsem_test_oauth_code_form_0123456789abcdef"}}).encode("utf-8"),
            headers={{"Content-Type": "application/x-www-form-urlencoded"}},
            method="POST",
        )
        with urllib.request.urlopen(form_token_req, timeout=30) as response:
            form_token = json.loads(response.read().decode("utf-8"))

        with urllib.request.urlopen(cfg["credential_response_url"], timeout=30) as response:
            credential_response = json.loads(response.read().decode("utf-8"))

        client = OpenAI(base_url=cfg["base_url"], api_key=cfg["credential_ref"])
        completion = client.chat.completions.create(
            model=cfg["model"],
            messages=[{{"role": "user", "content": "Replay the Capsem ironbank poem."}}],
        )
        message = completion.choices[0].message
        result = {{
            "echo_has_authorization": echo["has_authorization"],
            "echo_authorization_is_broker_ref": echo["authorization_is_broker_ref"],
            "query_echo_has_access_token": query_echo["query_has_access_token"],
            "query_echo_has_broker_ref": query_echo["query_has_broker_ref"],
            "json_token_kind": json_token["kind"],
            "form_token_kind": form_token["kind"],
            "credential_response_kind": credential_response["kind"],
            "model": completion.model,
            "content": message.content,
            "usage_total": completion.usage.total_tokens if completion.usage else 0,
        }}
        print("IRONBANK_BROKER_REPLAY_RESULT=" + json.dumps(result, sort_keys=True))
        """
    ).strip()


def _unknown_shape_probe_script(base_url: str) -> str:
    payload = {
        "url": f"{base_url.rstrip('/')}/model/shape",
        "api_key_parts": ["capsem_test_unknown_shape_", "key_0123456789abcdef"],
        "model": "gpt-4.1",
    }
    return textwrap.dedent(
        f"""
        import json
        import urllib.request

        cfg = json.loads({json.dumps(json.dumps(payload))})
        body = json.dumps({{
            "model": cfg["model"],
            "messages": [{{"role": "user", "content": "Classify this by body shape."}}],
            "tools": [{{
                "type": "function",
                "function": {{
                    "name": "fixture_lookup",
                    "parameters": {{
                        "type": "object",
                        "properties": {{"query": {{"type": "string"}}}},
                    }},
                }},
            }}],
        }}).encode("utf-8")
        request = urllib.request.Request(
            cfg["url"],
            data=body,
            headers={{
                "Authorization": "Bearer " + "".join(cfg["api_key_parts"]),
                "Content-Type": "application/json",
            }},
            method="POST",
        )
        with urllib.request.urlopen(request, timeout=30) as response:
            payload = json.loads(response.read().decode("utf-8"))
        result = {{
            "model": payload["model"],
            "content": payload["choices"][0]["message"]["content"],
            "tool_name": payload["choices"][0]["message"]["tool_calls"][0]["function"]["name"],
            "usage_total": payload["usage"]["total_tokens"],
        }}
        print("IRONBANK_UNKNOWN_SHAPE_RESULT=" + json.dumps(result, sort_keys=True))
        """
    ).strip()


def _tool_declaration_without_call_script(base_url: str) -> str:
    payload = {
        "url": f"{base_url.rstrip('/')}/model/no-tool-call",
        "api_key_parts": ["capsem_test_declared_tool_", "key_0123456789abcdef"],
        "model": "gpt-4.1",
    }
    return textwrap.dedent(
        f"""
        import json
        import urllib.request

        cfg = json.loads({json.dumps(json.dumps(payload))})
        body = json.dumps({{
            "model": cfg["model"],
            "messages": [{{"role": "user", "content": "Do not call a tool."}}],
            "tools": [{{
                "type": "function",
                "function": {{
                    "name": "fixture_lookup",
                    "parameters": {{
                        "type": "object",
                        "properties": {{"query": {{"type": "string"}}}},
                    }},
                }},
            }}],
        }}).encode("utf-8")
        request = urllib.request.Request(
            cfg["url"],
            data=body,
            headers={{
                "Authorization": "Bearer " + "".join(cfg["api_key_parts"]),
                "Content-Type": "application/json",
            }},
            method="POST",
        )
        with urllib.request.urlopen(request, timeout=30) as response:
            payload = json.loads(response.read().decode("utf-8"))
        message = payload["choices"][0]["message"]
        result = {{
            "model": payload["model"],
            "content": message["content"],
            "has_tool_calls": "tool_calls" in message,
            "finish_reason": payload["choices"][0]["finish_reason"],
            "usage_total": payload["usage"]["total_tokens"],
        }}
        print("IRONBANK_DECLARED_TOOL_ONLY_RESULT=" + json.dumps(result, sort_keys=True))
        """
    ).strip()


def _unknown_mcp_probe_script(base_url: str) -> str:
    payload = {"url": f"{base_url.rstrip('/')}/mcp"}
    return textwrap.dedent(
        f"""
        import json
        import urllib.request

        cfg = json.loads({json.dumps(json.dumps(payload))})

        def call_mcp(body):
            request = urllib.request.Request(
                cfg["url"],
                data=json.dumps(body).encode("utf-8"),
                headers={{"Content-Type": "application/json"}},
                method="POST",
            )
            with urllib.request.urlopen(request, timeout=30) as response:
                return json.loads(response.read().decode("utf-8"))

        initialize = call_mcp({{"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {{}}}})
        tools = call_mcp({{"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {{}}}})
        tool = call_mcp({{
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {{"name": "fixture_lookup", "arguments": {{"query": "capsem"}}}},
        }})
        result = {{
            "initialize_server": initialize["result"]["serverInfo"]["name"],
            "tool_count": len(tools["result"]["tools"]),
            "tool_text": tool["result"]["content"][0]["text"],
        }}
        print("IRONBANK_UNKNOWN_MCP_RESULT=" + json.dumps(result, sort_keys=True))
        """
    ).strip()


def test_openai_sdk_local_model_path_pays_full_ledger_debt_blackbox():
    assert MOCK_SERVER_BINARY.exists(), f"{MOCK_SERVER_BINARY} missing; restore mock server runtime"
    assert ASSETS_DIR.exists(), f"{ASSETS_DIR} missing; build VM assets before Ironbank"
    assert PROFILES_DIR.exists(), f"{PROFILES_DIR} missing; materialize profile config before Ironbank"

    service = ServiceInstance()
    client = None
    mock_proc = None
    session_id = vm_name("ironbank-sdk")
    script_name = f"ironbank-model-sdk-{uuid.uuid4().hex[:8]}.py"
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
        assert create is not None, "session creation returned no body"
        assert create.get("id") == session_id or create.get("name") == session_id
        assert wait_exec_ready(client, session_id, timeout=EXEC_READY_TIMEOUT)

        script = _sdk_probe_script(mock_base_url).encode()
        upload = client.post_bytes(
            f"/vms/{session_id}/files/content?path={script_name}",
            script,
            timeout=30,
        )
        assert upload is not None
        assert upload["success"] is True
        assert upload["size"] == len(script)

        exec_resp = client.post(
            f"/vms/{session_id}/exec",
            {"command": f"python3 /root/{script_name}", "timeout_secs": 220},
            timeout=240,
        )
        assert exec_resp is not None, "SDK exec returned no body"
        assert exec_resp["exit_code"] == 0, exec_resp
        stdout = exec_resp.get("stdout", "")
        stderr = exec_resp.get("stderr", "")
        assert RAW_SDK_SECRET not in stdout + stderr
        result_line = next(
            (line for line in stdout.splitlines() if line.startswith("IRONBANK_SDK_RESULT=")),
            None,
        )
        assert result_line is not None, stdout + stderr
        sdk_result = json.loads(result_line.split("=", 1)[1])
        assert sdk_result == {
            "first_content": EXPECTED_POEM,
            "first_model": "gemma4:latest",
            "first_tool_arguments": '{"query":"capsem"}',
            "first_tool_count": 1,
            "first_tool_name": "fixture_lookup",
            "poem_path": "/root/poem.md",
            "second_content": EXPECTED_POEM,
            "second_model": "gemma4:latest",
            "usage_total": 24,
        }

        poem_status, poem_bytes = client.get_bytes(
            f"/vms/{session_id}/files/content?path=poem.md",
            timeout=30,
        )
        assert poem_status == 200
        assert poem_bytes.decode() == EXPECTED_POEM + "\n"

        shape_script_name = f"ironbank-unknown-shape-{uuid.uuid4().hex[:8]}.py"
        shape_script = _unknown_shape_probe_script(mock_base_url).encode()
        shape_upload = client.post_bytes(
            f"/vms/{session_id}/files/content?path={shape_script_name}",
            shape_script,
            timeout=30,
        )
        assert shape_upload is not None
        assert shape_upload["success"] is True
        assert shape_upload["size"] == len(shape_script)
        shape_exec = client.post(
            f"/vms/{session_id}/exec",
            {"command": f"python3 /root/{shape_script_name}", "timeout_secs": 120},
            timeout=150,
        )
        assert shape_exec is not None, "unknown-shape exec returned no body"
        assert shape_exec["exit_code"] == 0, shape_exec
        shape_output = shape_exec.get("stdout", "") + shape_exec.get("stderr", "")
        assert "capsem_test_unknown_shape_key" not in shape_output
        shape_line = next(
            (
                line
                for line in shape_exec.get("stdout", "").splitlines()
                if line.startswith("IRONBANK_UNKNOWN_SHAPE_RESULT=")
            ),
            None,
        )
        assert shape_line is not None, shape_output
        shape_result = json.loads(shape_line.split("=", 1)[1])
        assert shape_result == {
            "content": EXPECTED_POEM,
            "model": "gpt-4.1",
            "tool_name": "fixture_lookup",
            "usage_total": 12,
        }

        declared_tool_script_name = f"ironbank-declared-tool-{uuid.uuid4().hex[:8]}.py"
        declared_tool_script = _tool_declaration_without_call_script(mock_base_url).encode()
        declared_tool_upload = client.post_bytes(
            f"/vms/{session_id}/files/content?path={declared_tool_script_name}",
            declared_tool_script,
            timeout=30,
        )
        assert declared_tool_upload is not None
        assert declared_tool_upload["success"] is True
        assert declared_tool_upload["size"] == len(declared_tool_script)
        declared_tool_exec = client.post(
            f"/vms/{session_id}/exec",
            {"command": f"python3 /root/{declared_tool_script_name}", "timeout_secs": 120},
            timeout=150,
        )
        assert declared_tool_exec is not None, "declared-tool exec returned no body"
        assert declared_tool_exec["exit_code"] == 0, declared_tool_exec
        declared_tool_output = (declared_tool_exec.get("stdout") or "") + (
            declared_tool_exec.get("stderr") or ""
        )
        assert "capsem_test_declared_tool_key" not in declared_tool_output
        declared_tool_line = next(
            (
                line
                for line in declared_tool_output.splitlines()
                if line.startswith("IRONBANK_DECLARED_TOOL_ONLY_RESULT=")
            ),
            None,
        )
        assert declared_tool_line is not None, declared_tool_output
        declared_tool_result = json.loads(declared_tool_line.split("=", 1)[1])
        assert declared_tool_result == {
            "content": EXPECTED_POEM,
            "finish_reason": "stop",
            "has_tool_calls": False,
            "model": "gpt-4.1",
            "usage_total": 12,
        }

        mcp_script_name = f"ironbank-unknown-mcp-{uuid.uuid4().hex[:8]}.py"
        mcp_script = _unknown_mcp_probe_script(mock_base_url).encode()
        mcp_upload = client.post_bytes(
            f"/vms/{session_id}/files/content?path={mcp_script_name}",
            mcp_script,
            timeout=30,
        )
        assert mcp_upload is not None
        assert mcp_upload["success"] is True
        assert mcp_upload["size"] == len(mcp_script)
        mcp_exec = client.post(
            f"/vms/{session_id}/exec",
            {"command": f"python3 /root/{mcp_script_name}", "timeout_secs": 120},
            timeout=150,
        )
        assert mcp_exec is not None, "unknown-MCP exec returned no body"
        assert mcp_exec["exit_code"] == 0, mcp_exec
        mcp_line = next(
            (
                line
                for line in mcp_exec.get("stdout", "").splitlines()
                if line.startswith("IRONBANK_UNKNOWN_MCP_RESULT=")
            ),
            None,
        )
        assert mcp_line is not None, mcp_exec.get("stdout", "") + mcp_exec.get("stderr", "")
        mcp_result = json.loads(mcp_line.split("=", 1)[1])
        assert mcp_result == {
            "initialize_server": "capsem-mock-server",
            "tool_count": 3,
            "tool_text": "capsem-mock-server:mcp:fixture_lookup",
        }

        history = client.get(f"/vms/{session_id}/history", timeout=30)
        assert history is not None
        assert history.get("total", 0) >= 2
        history_text = " ".join(
            (entry.get("command") or "") + " " + (entry.get("stdout_preview") or "")
            for entry in history.get("commands", [])
        )
        assert script_name in history_text
        assert "IRONBANK_SDK_RESULT" in history_text
        assert RAW_SDK_SECRET not in history_text

        security_latest = client.get(f"/vms/{session_id}/security/latest?limit=50", timeout=30)
        assert isinstance(security_latest, list)
        assert security_latest
        assert all(set(row) == EXPECTED_SECURITY_LATEST_FIELDS for row in security_latest)
        assert any(row["event_type"] == "model.call" for row in security_latest)
        assert any(row["event_type"] == "http.request" for row in security_latest)
        assert all(row["rule_action"] in {"allow", "ask", "block", "preprocess", "rewrite", "postprocess"} for row in security_latest)
        assert all(row["detection_level"] in {"none", "informational", "low", "medium", "high", "critical"} for row in security_latest)
        assert all(json.loads(row["rule_json"]) for row in security_latest)
        assert all(json.loads(row["event_json"]) for row in security_latest)

        conn = _connect_session_db(service, session_id)
        try:
            for table in (
                "net_events",
                "model_calls",
                "tool_calls",
                "fs_events",
                "exec_events",
                "security_rule_events",
                "substitution_events",
            ):
                assert "event_id" in _table_columns(conn, table), f"{table} lacks event_id"

            net_rows = _eventually(
                lambda: conn.execute(
                    """
                    SELECT *
                    FROM net_events
                    WHERE path = '/v1/chat/completions'
                    ORDER BY id
                    """
                ).fetchall(),
                lambda rows: len(rows) >= 2,
            )
            assert len(net_rows) >= 2
            credential_refs = {row["credential_ref"] for row in net_rows}
            assert len(credential_refs) == 1
            credential_ref = next(iter(credential_refs))
            _assert_credential_ref(credential_ref)

            replay_script_name = f"ironbank-broker-replay-{uuid.uuid4().hex[:8]}.py"
            replay_script = _broker_replay_script(mock_base_url, credential_ref).encode()
            replay_upload = client.post_bytes(
                f"/vms/{session_id}/files/content?path={replay_script_name}",
                replay_script,
                timeout=30,
            )
            assert replay_upload is not None
            assert replay_upload["success"] is True
            assert replay_upload["size"] == len(replay_script)

            replay_exec = client.post(
                f"/vms/{session_id}/exec",
                {"command": f"python3 /root/{replay_script_name}", "timeout_secs": 220},
                timeout=240,
            )
            assert replay_exec is not None
            assert replay_exec["exit_code"] == 0, replay_exec
            replay_output = (replay_exec.get("stdout") or "") + (replay_exec.get("stderr") or "")
            assert RAW_SDK_SECRET not in replay_output
            replay_line = next(
                (
                    line
                    for line in replay_output.splitlines()
                    if line.startswith("IRONBANK_BROKER_REPLAY_RESULT=")
                ),
                None,
            )
            assert replay_line is not None, replay_output
            replay_result = json.loads(replay_line.split("=", 1)[1])
            assert replay_result == {
                "content": EXPECTED_POEM,
                "credential_response_kind": "synthetic_credential_fixture",
                "echo_authorization_is_broker_ref": False,
                "echo_has_authorization": True,
                "form_token_kind": "synthetic_oauth_token_fixture",
                "json_token_kind": "synthetic_oauth_token_fixture",
                "model": "gemma4:latest",
                "query_echo_has_access_token": True,
                "query_echo_has_broker_ref": False,
                "usage_total": 12,
            }

            net_rows = _eventually(
                lambda: conn.execute(
                    """
                    SELECT *
                    FROM net_events
                    WHERE path = '/v1/chat/completions'
                    ORDER BY id
                    """
                ).fetchall(),
                lambda rows: len(rows) >= 3,
            )
            for row in net_rows:
                _assert_event_id(row["event_id"])
                assert row["method"] == "POST"
                assert row["status_code"] == 200
                assert row["decision"] == "allowed"
                assert row["domain"] == "127.0.0.1"
                assert row["port"] == 3713
                assert row["bytes_sent"] > 0
                assert row["bytes_received"] > 0
                assert row["trace_id"]
                assert RAW_SDK_SECRET not in (row["request_headers"] or "")
                assert RAW_SDK_SECRET not in (row["request_body_preview"] or "")
                assert EXPECTED_POEM.splitlines()[0] in (row["response_body_preview"] or "")

            echo_rows = _eventually(
                lambda: conn.execute(
                    """
                    SELECT *
                    FROM net_events
                    WHERE path = '/echo'
                    ORDER BY id
                    """
                ).fetchall(),
                lambda rows: len(rows) >= 1,
            )
            replay_echo = next(row for row in echo_rows if not row["query"])
            _assert_event_id(replay_echo["event_id"])
            assert replay_echo["credential_ref"] == credential_ref
            assert replay_echo["decision"] == "allowed"
            assert replay_echo["status_code"] == 200
            assert RAW_SDK_SECRET not in (replay_echo["request_headers"] or "")
            assert credential_ref not in (replay_echo["request_headers"] or "")
            assert "authorization: hash:" in (replay_echo["request_headers"] or "")
            assert '"authorization_is_broker_ref":false' in (
                replay_echo["response_body_preview"] or ""
            )

            query_echo = next(
                row for row in echo_rows if row["query"] and "access_token=" in row["query"]
            )
            assert query_echo["credential_ref"] == credential_ref
            assert credential_ref not in (query_echo["query"] or "")
            assert RAW_SDK_SECRET not in (query_echo["query"] or "")
            assert '"query_has_broker_ref":false' in (query_echo["response_body_preview"] or "")

            token_rows = _eventually(
                lambda: conn.execute(
                    """
                    SELECT *
                    FROM net_events
                    WHERE path = '/oauth/token'
                    ORDER BY id
                    """
                ).fetchall(),
                lambda rows: len(rows) >= 2,
            )
            for row in token_rows:
                _assert_event_id(row["event_id"])
                assert row["method"] == "POST"
                assert row["status_code"] == 200
                assert row["decision"] == "allowed"
                assert row["credential_ref"] is not None
                _assert_credential_ref(row["credential_ref"])
                assert "capsem_test_oauth_access_json_" not in (row["request_body_preview"] or "")
                assert "capsem_test_oauth_code_form_" not in (row["request_body_preview"] or "")
                assert "capsem_test_oauth_access_" not in (row["response_body_preview"] or "")
                assert "capsem_test_oauth_refresh_" not in (row["response_body_preview"] or "")
                assert "capsem_test_oauth_id_" not in (row["response_body_preview"] or "")
                assert "credential:blake3:" in (row["request_body_preview"] or "") or "credential:blake3:" in (row["response_body_preview"] or "")

            credential_response_rows = _eventually(
                lambda: conn.execute(
                    """
                    SELECT *
                    FROM net_events
                    WHERE path = '/credential/response'
                    ORDER BY id
                    """
                ).fetchall(),
                lambda rows: len(rows) >= 1,
            )
            credential_response = credential_response_rows[-1]
            _assert_event_id(credential_response["event_id"])
            assert credential_response["status_code"] == 200
            assert credential_response["credential_ref"] is not None
            _assert_credential_ref(credential_response["credential_ref"])
            assert "capsem_test_api_key_" not in (credential_response["response_body_preview"] or "")
            assert "capsem_test_oauth_access_" not in (credential_response["response_body_preview"] or "")
            assert "capsem_test_oauth_refresh_" not in (credential_response["response_body_preview"] or "")
            assert "credential:blake3:" in (credential_response["response_body_preview"] or "")

            model_rows = _eventually(
                lambda: conn.execute(
                    """
                    SELECT *
                    FROM model_calls
                    WHERE path = '/v1/chat/completions'
                    ORDER BY id
                    """
                ).fetchall(),
                lambda rows: len(rows) >= 3,
            )
            assert len(model_rows) >= 3
            model_trace_ids = {row["trace_id"] for row in model_rows}
            net_trace_ids = {row["trace_id"] for row in net_rows}
            assert model_trace_ids <= net_trace_ids
            for row in model_rows:
                _assert_event_id(row["event_id"])
                assert row["provider"] == "openai"
                assert row["model"] == "gemma4:latest"
                assert row["method"] == "POST"
                assert row["status_code"] == 200
                assert row["messages_count"] >= 1
                assert row["tools_count"] in {0, 1}
                assert row["request_bytes"] > 0
                assert row["input_tokens"] == 7
                assert row["output_tokens"] == 5
                assert row["response_bytes"] > 0
                assert row["text_content"] == EXPECTED_POEM
                assert row["stop_reason"] == "tool_use"
                assert row["credential_ref"] == credential_ref
                assert RAW_SDK_SECRET not in (row["request_body_preview"] or "")

            unknown_shape_rows = _eventually(
                lambda: conn.execute(
                    """
                    SELECT *
                    FROM model_calls
                    WHERE path = '/model/shape'
                    ORDER BY id
                    """
                ).fetchall(),
                lambda rows: len(rows) >= 1,
            )
            unknown_shape = unknown_shape_rows[-1]
            _assert_event_id(unknown_shape["event_id"])
            assert unknown_shape["provider"] == "openai"
            assert unknown_shape["model"] == "gpt-4.1"
            assert unknown_shape["method"] == "POST"
            assert unknown_shape["status_code"] == 200
            assert unknown_shape["messages_count"] == 1
            assert unknown_shape["tools_count"] == 1
            assert unknown_shape["input_tokens"] == 7
            assert unknown_shape["output_tokens"] == 5
            assert unknown_shape["text_content"] == EXPECTED_POEM
            assert unknown_shape["credential_ref"] is not None
            _assert_credential_ref(unknown_shape["credential_ref"])
            assert "capsem_test_unknown_shape_key" not in (
                unknown_shape["request_body_preview"] or ""
            )

            declared_tool_only_rows = _eventually(
                lambda: conn.execute(
                    """
                    SELECT *
                    FROM model_calls
                    WHERE path = '/model/no-tool-call'
                    ORDER BY id
                    """
                ).fetchall(),
                lambda rows: len(rows) >= 1,
            )
            declared_tool_only = declared_tool_only_rows[-1]
            _assert_event_id(declared_tool_only["event_id"])
            assert declared_tool_only["provider"] == "openai"
            assert declared_tool_only["model"] == "gpt-4.1"
            assert declared_tool_only["method"] == "POST"
            assert declared_tool_only["status_code"] == 200
            assert declared_tool_only["messages_count"] == 1
            assert declared_tool_only["tools_count"] == 1
            assert declared_tool_only["text_content"] == EXPECTED_POEM
            assert declared_tool_only["stop_reason"] == "end_turn"
            assert declared_tool_only["credential_ref"] is not None
            _assert_credential_ref(declared_tool_only["credential_ref"])
            assert "capsem_test_declared_tool_key" not in (
                declared_tool_only["request_body_preview"] or ""
            )
            declared_tool_call_rows = conn.execute(
                """
                SELECT *
                FROM tool_calls
                WHERE model_call_id = ?
                """,
                (declared_tool_only["id"],),
            ).fetchall()
            assert declared_tool_call_rows == []

            tool_rows = _eventually(
                lambda: conn.execute(
                    """
                    SELECT tool_calls.*, model_calls.trace_id AS model_trace_id
                    FROM tool_calls
                    JOIN model_calls ON model_calls.id = tool_calls.model_call_id
                    WHERE tool_calls.tool_name = 'fixture_lookup'
                    ORDER BY tool_calls.id
                    """
                ).fetchall(),
                lambda rows: len(rows) >= 2,
            )
            assert len(tool_rows) >= 2
            assert {row["call_id"] for row in tool_rows} == {"tool_0001"}
            valid_tool_credential_refs = {
                credential_ref,
                unknown_shape["credential_ref"],
            }
            for row in tool_rows:
                _assert_event_id(row["event_id"])
                assert row["provider"] == "openai"
                assert row["status"] == "observed"
                assert row["call_index"] == 0
                assert row["arguments"] == '{"query":"capsem"}'
                assert row["origin"] == "native"
                assert row["trace_id"] == row["model_trace_id"]
                _assert_credential_ref(row["credential_ref"])
                assert row["credential_ref"] in valid_tool_credential_refs

            observed_mcp_server = "observed:127.0.0.1:3713/mcp"
            observed_mcp_rows = _eventually(
                lambda: conn.execute(
                    """
                    SELECT *
                    FROM mcp_calls
                    WHERE server_name = ?
                    ORDER BY id
                    """,
                    (observed_mcp_server,),
                ).fetchall(),
                lambda rows: len(rows) >= 3,
            )
            observed_methods = {row["method"] for row in observed_mcp_rows}
            assert {"initialize", "tools/list", "tools/call"} <= observed_methods
            observed_tool_call = next(
                row for row in observed_mcp_rows if row["method"] == "tools/call"
            )
            _assert_event_id(observed_tool_call["event_id"])
            assert observed_tool_call["tool_name"] == "fixture_lookup"
            assert observed_tool_call["decision"] == "allowed"
            assert observed_tool_call["bytes_sent"] > 0
            assert observed_tool_call["bytes_received"] > 0
            assert "fixture_lookup" in (observed_tool_call["request_preview"] or "")
            assert "capsem-mock-server:mcp:fixture_lookup" in (
                observed_tool_call["response_preview"] or ""
            )
            observed_tool_list = next(
                row for row in observed_mcp_rows if row["method"] == "tools/list"
            )
            _assert_event_id(observed_tool_list["event_id"])
            assert "fixture_lookup" in (observed_tool_list["response_preview"] or "")

            timeline = client.get(f"/vms/{session_id}/timeline?layers=mcp&limit=50", timeout=30)
            assert set(timeline) == {"columns", "rows"}
            assert {"timestamp", "layer", "ref", "summary", "status", "duration_ms"} <= set(
                timeline["columns"]
            )
            timeline_rows = [
                dict(zip(timeline["columns"], row, strict=True)) for row in timeline["rows"]
            ]
            timeline_summaries = {row["summary"] for row in timeline_rows}
            assert f"{observed_mcp_server}/fixture_lookup" in timeline_summaries
            assert f"{observed_mcp_server}/tools/list" in timeline_summaries

            info = _eventually(
                lambda: client.get(f"/vms/{session_id}/info", timeout=30),
                lambda value: (
                    value is not None
                    and (value.get("id") == session_id or value.get("name") == session_id)
                    and value.get("model_call_count", 0) >= len(model_rows)
                    and value.get("total_tool_calls", 0) >= len(tool_rows)
                ),
                timeout_s=20,
            )
            assert info["profile_id"] == CODE_PROFILE_ID
            assert info["model_call_count"] >= len(model_rows)
            assert info["total_tool_calls"] >= len(tool_rows)
            status = client.get(f"/vms/{session_id}/status", timeout=30)
            assert status is not None
            assert status["status"] == "Running"
            assert status["available_actions"] == ["pause", "stop", "fork", "delete"]

            security_rows = conn.execute(
                """
                SELECT *
                FROM security_rule_events
                WHERE event_id IN (
                    SELECT event_id FROM model_calls WHERE path = '/v1/chat/completions'
                    UNION
                    SELECT event_id FROM model_calls WHERE path = '/model/shape'
                    UNION
                    SELECT event_id FROM model_calls WHERE path = '/model/no-tool-call'
                    UNION
                    SELECT event_id FROM mcp_calls WHERE server_name = 'observed:127.0.0.1:3713/mcp'
                    UNION
                    SELECT event_id FROM net_events WHERE path = '/v1/chat/completions'
                )
                ORDER BY id
                """
            ).fetchall()
            assert security_rows
            assert {"http.request", "model.call", "mcp.tool_call", "mcp.tool_list"} <= {
                row["event_type"] for row in security_rows
            }
            assert all(json.loads(row["rule_json"]) for row in security_rows)
            assert all(json.loads(row["event_json"]) for row in security_rows)
            security_by_event: dict[str, list[sqlite3.Row]] = {}
            for row in security_rows:
                security_by_event.setdefault(row["event_id"], []).append(row)
            for row in net_rows:
                rows = security_by_event[row["event_id"]]
                rule_ids = {item["rule_id"] for item in rows}
                actions = {item["rule_action"] for item in rows}
                assert "allow" in actions
                assert "profiles.rules.default_http" in rule_ids
                assert "profiles.rules.ai_ollama_http_local_host" in rule_ids
                assert "profiles.rules.default_000_local_network" in rule_ids
                assert any(
                    item["rule_id"] == "profiles.rules.default_000_local_network"
                    and item["rule_action"] == "ask"
                    for item in rows
                )
            for row in model_rows:
                rows = security_by_event[row["event_id"]]
                assert {item["rule_action"] for item in rows} == {"allow"}
                assert {
                    "profiles.rules.ai_openai_model_api",
                    "profiles.rules.default_model",
                } <= {item["rule_id"] for item in rows}
            shape_security_rows = security_by_event[unknown_shape["event_id"]]
            assert {item["rule_action"] for item in shape_security_rows} == {"allow"}
            assert {
                "profiles.rules.ai_openai_model_api",
                "profiles.rules.default_model",
            } <= {item["rule_id"] for item in shape_security_rows}
            assert any(
                item["rule_id"] == "profiles.rules.ai_openai_model_api"
                and item["detection_level"] == "informational"
                for item in shape_security_rows
            )
            declared_tool_security_rows = security_by_event[declared_tool_only["event_id"]]
            assert {item["rule_action"] for item in declared_tool_security_rows} == {"allow"}
            assert {
                "profiles.rules.ai_openai_model_api",
                "profiles.rules.default_model",
            } <= {item["rule_id"] for item in declared_tool_security_rows}
            mcp_tool_security_rows = security_by_event[observed_tool_call["event_id"]]
            assert any(
                item["event_type"] == "mcp.tool_call"
                and item["rule_id"] == "profiles.rules.default_mcp"
                and item["rule_action"] in {"allow", "ask"}
                for item in mcp_tool_security_rows
            )
            mcp_list_security_rows = security_by_event[observed_tool_list["event_id"]]
            assert any(
                item["event_type"] == "mcp.tool_list"
                and item["rule_id"] == "profiles.rules.default_mcp"
                and item["rule_action"] in {"allow", "ask"}
                for item in mcp_list_security_rows
            )
            security_payloads = [json.loads(row["event_json"]) for row in security_rows]
            plugin_executions = [
                execution
                for payload in security_payloads
                for execution in payload.get("plugin_executions", [])
            ]
            assert plugin_executions, "security ledger must carry plugin execution counters"
            assert {
                "plugin_id",
                "stage",
                "applied",
                "duration_us",
            } <= plugin_executions[0].keys()
            assert all(
                execution["stage"] in {"preprocess", "postprocess", "logging"}
                for execution in plugin_executions
            )
            assert all(isinstance(execution["applied"], bool) for execution in plugin_executions)
            assert all(isinstance(execution["duration_us"], int) for execution in plugin_executions)
            assert any(
                execution["plugin_id"] == "credential_broker"
                for execution in plugin_executions
            )
            assert any(
                execution["plugin_id"] == "log_sanitizer" and execution["applied"] is True
                for execution in plugin_executions
            )
            assert any(
                detection.get("source") == "plugin"
                and detection.get("plugin_id") == "log_sanitizer"
                for payload in security_payloads
                for detection in payload.get("detections", [])
            )

            plugins = client.get(f"/profiles/{CODE_PROFILE_ID}/plugins/list", timeout=30)
            assert plugins is not None
            by_plugin = {plugin["id"]: plugin for plugin in plugins["plugins"]}
            broker_runtime = by_plugin["credential_broker"]["runtime"]
            sanitizer_runtime = by_plugin["log_sanitizer"]["runtime"]
            for runtime in (broker_runtime, sanitizer_runtime):
                assert runtime["enabled"] is True
                assert runtime["execution_count"] > 0
                assert runtime["applied_count"] + runtime["skipped_count"] == runtime["execution_count"]
                assert runtime["total_duration_us"] >= runtime["max_duration_us"]
                assert runtime["max_duration_us"] >= 0
            assert broker_runtime["applied_count"] > 0
            assert broker_runtime["detection_count"] > 0
            assert sanitizer_runtime["applied_count"] > 0
            assert sanitizer_runtime["detection_count"] > 0

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
            assert {"captured", "brokered", "injected"} <= {
                row["outcome"] for row in substitutions
            }
            assert all(row["material_class"] == "credential" for row in substitutions)
            assert all(row["algorithm"] == "blake3" for row in substitutions)
            assert all(row["substitution_ref"] == credential_ref for row in substitutions)
            assert all(row["event_type"] == "http.request" for row in substitutions)
            assert len(substitutions) >= len(net_rows)
            injected_sources = {row["source"] for row in substitutions if row["outcome"] == "injected"}
            assert "http.header.authorization" in injected_sources
            assert "http.query.access_token" in injected_sources

            body_substitutions = conn.execute(
                """
                SELECT *
                FROM substitution_events
                WHERE source LIKE 'http.body.%'
                ORDER BY id
                """
            ).fetchall()
            sources = {row["source"] for row in body_substitutions}
            assert "http.body.request.$.access_token" in sources
            assert "http.body.request.form.code" in sources
            assert "http.body.response.$.access_token" in sources
            assert "http.body.response.$.refresh_token" in sources
            assert "http.body.response.$.id_token" in sources
            assert "http.body.response.$.api_key" in sources
            assert {row["outcome"] for row in body_substitutions} == {
                "captured",
                "brokered",
            }
            assert all(row["substitution_ref"].startswith("credential:blake3:") for row in body_substitutions)

            poem_rows = _eventually(
                lambda: conn.execute(
                    "SELECT * FROM fs_events WHERE path = 'poem.md' ORDER BY id"
                ).fetchall(),
                lambda rows: any(row["action"] in {"created", "modified"} for row in rows),
            )
            assert poem_rows
            assert any(row["action"] in {"created", "modified"} for row in poem_rows)
            assert all(row["size"] is None or row["size"] >= len(EXPECTED_POEM) for row in poem_rows)
            assert all(row["credential_ref"] is None for row in poem_rows)

            exec_row = conn.execute(
                "SELECT * FROM exec_events WHERE command = ? ORDER BY id DESC LIMIT 1",
                (f"python3 /root/{script_name}",),
            ).fetchone()
            assert exec_row is not None
            _assert_event_id(exec_row["event_id"])
            assert exec_row["exit_code"] == 0
            assert exec_row["source"] == "api"
            assert "IRONBANK_SDK_RESULT" in (exec_row["stdout_preview"] or "")
            assert RAW_SDK_SECRET not in (exec_row["stdout_preview"] or "")
            assert exec_row["credential_ref"] is None

            _assert_raw_secret_not_in_db(conn)
        finally:
            conn.close()
    finally:
        stop_process(mock_proc)
        if client is not None:
            try:
                client.delete(f"/vms/{session_id}/delete", timeout=60)
            except Exception:
                pass
        service.stop()
