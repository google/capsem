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
RAW_CODEX_SECRET = "capsem_test_codex_cli_key_0123456789abcdef"
RAW_CODEX_BROKER_SECRET = "sk-capsem-test-codex-cli-key-0123456789abcdef"
EXPECTED_POEM = "Capsem ironbank poem\nledgers count the sparks\nno secret crosses raw"
CODEX_NO_SIDE_TRAFFIC_CONFIG = """

check_for_update_on_startup = false

[analytics]
enabled = false

[otel]
exporter = "none"
metrics_exporter = "none"
trace_exporter = "none"

[features]
plugins = false
plugin_sharing = false
"""
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
    raw_secrets = [RAW_SDK_SECRET, RAW_CODEX_SECRET, RAW_CODEX_BROKER_SECRET]
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
                for raw_secret in raw_secrets:
                    assert raw_secret not in str(value), (
                        f"raw secret leaked in {table}.{column}"
                    )


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


def _streaming_provider_probe_script(base_url: str) -> str:
    payload = {
        "google_url": f"{base_url.rstrip('/')}/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse",
        "anthropic_url": f"{base_url.rstrip('/')}/v1/messages",
        "google_key_parts": ["capsem_test_google_stream_", "key_0123456789abcdef"],
        "anthropic_key_parts": ["capsem_test_anthropic_stream_", "key_0123456789abcdef"],
    }
    return textwrap.dedent(
        f"""
        import json
        import urllib.request

        cfg = json.loads({json.dumps(json.dumps(payload))})

        def post(url, body, headers):
            request = urllib.request.Request(
                url,
                data=json.dumps(body).encode("utf-8"),
                headers={{"Content-Type": "application/json", **headers}},
                method="POST",
            )
            with urllib.request.urlopen(request, timeout=30) as response:
                return {{
                    "status": response.status,
                    "content_type": response.headers.get("content-type"),
                    "body": response.read().decode("utf-8"),
                }}

        google = post(
            cfg["google_url"],
            {{"contents": [{{"parts": [{{"text": "stream a greeting"}}]}}]}},
            {{"x-goog-api-key": "".join(cfg["google_key_parts"])}},
        )
        anthropic = post(
            cfg["anthropic_url"],
            {{
                "model": "claude-sonnet-4-20250514",
                "max_tokens": 32,
                "stream": True,
                "messages": [{{"role": "user", "content": "stream a greeting"}}],
            }},
            {{
                "x-api-key": "".join(cfg["anthropic_key_parts"]),
                "anthropic-version": "2023-06-01",
            }},
        )
        result = {{
            "google_status": google["status"],
            "google_content_type": google["content_type"],
            "google_bytes": len(google["body"].encode("utf-8")),
            "google_has_text": "Hello" in google["body"] and "world!" in google["body"],
            "anthropic_status": anthropic["status"],
            "anthropic_content_type": anthropic["content_type"],
            "anthropic_bytes": len(anthropic["body"].encode("utf-8")),
            "anthropic_has_text": "Hello" in anthropic["body"] and "world!" in anthropic["body"],
        }}
        print("IRONBANK_STREAMING_PROVIDER_RESULT=" + json.dumps(result, sort_keys=True))
        """
    ).strip()


def _real_client_diversity_probe_script(base_url: str) -> str:
    payload = {
        "base_url": base_url.rstrip("/"),
        "openai_base_url": f"{base_url.rstrip('/')}/v1",
        "poem_paths": {
            "anthropic": "/root/anthropic-sdk-poem.md",
            "litellm": "/root/litellm-poem.md",
            "ollama": "/root/ollama-sdk-poem.md",
        },
        "secrets": {
            "anthropic": ["capsem_test_anthropic_sdk_", "key_0123456789abcdef"],
            "litellm": ["capsem_test_litellm_sdk_", "key_0123456789abcdef"],
            "ollama": ["capsem_test_ollama_sdk_", "key_0123456789abcdef"],
        },
    }
    return textwrap.dedent(
        f"""
        import json
        import os
        from pathlib import Path

        os.environ["LITELLM_LOCAL_MODEL_COST_MAP"] = "True"

        import anthropic
        import litellm
        import ollama

        cfg = json.loads({json.dumps(json.dumps(payload))})
        litellm.register_model({{
            "openai/gemma4:latest": {{
                "max_tokens": 8192,
                "max_input_tokens": 8192,
                "max_output_tokens": 8192,
                "input_cost_per_token": 0.0,
                "output_cost_per_token": 0.0,
                "litellm_provider": "openai",
                "mode": "chat",
            }},
            "gemma4:latest": {{
                "max_tokens": 8192,
                "max_input_tokens": 8192,
                "max_output_tokens": 8192,
                "input_cost_per_token": 0.0,
                "output_cost_per_token": 0.0,
                "litellm_provider": "openai",
                "mode": "chat",
            }},
        }})

        anthropic_client = anthropic.Anthropic(
            base_url=cfg["base_url"],
            api_key="".join(cfg["secrets"]["anthropic"]),
        )
        anthropic_message = anthropic_client.messages.create(
            model="claude-sonnet-4-20250514",
            max_tokens=64,
            messages=[{{"role": "user", "content": "Write the Capsem ironbank poem."}}],
        )
        anthropic_text = "".join(
            block.text for block in anthropic_message.content if getattr(block, "type", None) == "text"
        )
        Path(cfg["poem_paths"]["anthropic"]).write_text(anthropic_text + "\\n", encoding="utf-8")

        litellm_response = litellm.completion(
            model="openai/gemma4:latest",
            api_base=cfg["openai_base_url"],
            api_key="".join(cfg["secrets"]["litellm"]),
            messages=[{{"role": "user", "content": "Write the Capsem ironbank poem."}}],
        )
        litellm_text = litellm_response.choices[0].message.content
        Path(cfg["poem_paths"]["litellm"]).write_text(litellm_text + "\\n", encoding="utf-8")

        ollama_client = ollama.Client(host=cfg["base_url"])
        ollama_response = ollama_client.chat(
            model="gemma4:latest",
            messages=[{{"role": "user", "content": "Write the Capsem ironbank poem."}}],
            stream=False,
        )
        ollama_text = ollama_response["message"]["content"]
        Path(cfg["poem_paths"]["ollama"]).write_text(ollama_text + "\\n", encoding="utf-8")

        result = {{
            "anthropic_model": anthropic_message.model,
            "anthropic_text": anthropic_text,
            "anthropic_usage_total": anthropic_message.usage.input_tokens
                + anthropic_message.usage.output_tokens,
            "litellm_model": litellm_response.model,
            "litellm_text": litellm_text,
            "litellm_usage_total": litellm_response.usage.total_tokens,
            "ollama_model": ollama_response["model"],
            "ollama_text": ollama_text,
            "ollama_prompt_eval_count": ollama_response["prompt_eval_count"],
            "ollama_eval_count": ollama_response["eval_count"],
            "poem_paths": cfg["poem_paths"],
        }}
        print("IRONBANK_REAL_CLIENT_RESULT=" + json.dumps(result, sort_keys=True))
        """
    ).strip()


def _codex_cli_probe_script(base_url: str) -> str:
    payload = {
        "openai_base_url": f"{base_url.rstrip('/')}/v1",
        "echo_url": f"{base_url.rstrip('/')}/echo",
        "codex_config": "/root/.codex/config.toml",
        "api_key_parts": ["capsem_test_codex_cli_", "key_0123456789abcdef"],
        "broker_key_parts": ["sk-capsem-test-codex-cli-", "key-0123456789abcdef"],
    }
    return textwrap.dedent(
        f"""
        import json
        import os
        import subprocess
        import urllib.request
        import uuid
        from pathlib import Path

        cfg = json.loads({json.dumps(json.dumps(payload))})
        codex_config = Path(cfg["codex_config"])
        codex_text = codex_config.read_text(encoding="utf-8")
        codex_text = codex_text.replace(
            'base_url = "http://127.0.0.1:11434/v1"',
            'base_url = "' + cfg["openai_base_url"] + '"',
        )
        if "check_for_update_on_startup" not in codex_text:
            codex_text += {json.dumps(CODEX_NO_SIDE_TRAFFIC_CONFIG)}
        codex_config.write_text(codex_text, encoding="utf-8")

        env = os.environ.copy()
        env["HOME"] = "/root"
        env["NO_COLOR"] = "1"
        env["TERM"] = "xterm-256color"
        env["OPENAI_API_KEY"] = "".join(cfg["api_key_parts"])

        broker_secret = "".join(cfg["broker_key_parts"])
        broker_req = urllib.request.Request(
            cfg["echo_url"],
            data=b"codex broker probe",
            headers={{
                "Authorization": "Bearer " + broker_secret,
                "Content-Type": "text/plain",
            }},
            method="POST",
        )
        with urllib.request.urlopen(broker_req, timeout=30) as response:
            broker_echo = json.loads(response.read().decode("utf-8"))

        nonce = uuid.uuid4().hex
        filename = "codex-cli-" + uuid.uuid4().hex + ".txt"
        target_path = "/root/" + filename
        prompt = (
            "Write uuid4 hex value " + nonce + " to " + target_path + "."
        )
        completed = subprocess.run(
            [
                "codex",
                "exec",
                "--dangerously-bypass-approvals-and-sandbox",
                "--skip-git-repo-check",
                "--cd",
                "/root",
                prompt,
            ],
            cwd="/root",
            env=env,
            text=True,
            capture_output=True,
            timeout=180,
        )
        output = (completed.stdout or "") + (completed.stderr or "")
        if completed.returncode != 0:
            raise SystemExit("codex failed with " + str(completed.returncode) + "\\n" + output)
        poem_path = Path(target_path)
        if not poem_path.exists():
            raise SystemExit("codex completed without writing " + target_path + "\\n" + output)
        poem_text = poem_path.read_text(encoding="utf-8")
        result = {{
            "broker_echo": broker_echo,
            "contains_nonce": nonce in output,
            "file_contains_nonce": poem_text == nonce + "\\n",
            "filename": filename,
            "nonce": nonce,
            "output_bytes": len(output.encode("utf-8")),
            "poem_bytes": len(poem_text.encode("utf-8")),
            "poem_path": target_path,
        }}
        print("IRONBANK_CODEX_CLI_RESULT=" + json.dumps(result, sort_keys=True))
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
        mock_proc, ready = start_mock_server(
            request_log=service.tmp_dir / "mock-server-requests.jsonl"
        )
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
            "first_tool_arguments": '{"query":"Capsem ironbank poem"}',
            "first_tool_count": 1,
            "first_tool_name": "fixture_lookup",
            "poem_path": "/root/poem.md",
            "second_content": EXPECTED_POEM,
            "second_model": "gemma4:latest",
            "usage_total": 534,
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
            "usage_total": 456,
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
            "usage_total": 78,
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

        streaming_script_name = f"ironbank-streaming-{uuid.uuid4().hex[:8]}.py"
        streaming_script = _streaming_provider_probe_script(mock_base_url).encode()
        streaming_upload = client.post_bytes(
            f"/vms/{session_id}/files/content?path={streaming_script_name}",
            streaming_script,
            timeout=30,
        )
        assert streaming_upload is not None
        assert streaming_upload["success"] is True
        assert streaming_upload["size"] == len(streaming_script)
        streaming_exec = client.post(
            f"/vms/{session_id}/exec",
            {"command": f"python3 /root/{streaming_script_name}", "timeout_secs": 120},
            timeout=150,
        )
        assert streaming_exec is not None, "streaming provider exec returned no body"
        assert streaming_exec["exit_code"] == 0, streaming_exec
        streaming_output = (streaming_exec.get("stdout") or "") + (
            streaming_exec.get("stderr") or ""
        )
        assert "capsem_test_google_stream_key" not in streaming_output
        assert "capsem_test_anthropic_stream_key" not in streaming_output
        streaming_line = next(
            (
                line
                for line in streaming_output.splitlines()
                if line.startswith("IRONBANK_STREAMING_PROVIDER_RESULT=")
            ),
            None,
        )
        assert streaming_line is not None, streaming_output
        streaming_result = json.loads(streaming_line.split("=", 1)[1])
        assert streaming_result["google_status"] == 200
        assert streaming_result["anthropic_status"] == 200
        assert "text/event-stream" in streaming_result["google_content_type"]
        assert "text/event-stream" in streaming_result["anthropic_content_type"]
        assert streaming_result["google_bytes"] > 100
        assert streaming_result["anthropic_bytes"] > 100
        assert streaming_result["google_has_text"] is True
        assert streaming_result["anthropic_has_text"] is True

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
                "usage_total": 78,
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
                response_preview = row["response_body_preview"] or ""
                if '"tool_calls"' in response_preview:
                    assert '"finish_reason":"tool_calls"' in response_preview
                else:
                    assert EXPECTED_POEM.splitlines()[0] in response_preview

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
                if row["tools_count"] == 1:
                    assert row["input_tokens"] == 66
                    assert row["output_tokens"] == 390
                    assert row["text_content"] in {"", None}
                    assert row["stop_reason"] == "tool_use"
                else:
                    assert row["input_tokens"] == 26
                    assert row["output_tokens"] == 52
                    assert row["text_content"] == EXPECTED_POEM
                    assert row["stop_reason"] == "end_turn"
                assert row["response_bytes"] > 0
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
            assert unknown_shape["provider"] == "unknown"
            assert unknown_shape["model"] == "gpt-4.1"
            assert unknown_shape["method"] == "POST"
            assert unknown_shape["status_code"] == 200
            assert unknown_shape["messages_count"] == 1
            assert unknown_shape["tools_count"] == 1
            assert unknown_shape["input_tokens"] == 66
            assert unknown_shape["output_tokens"] == 390
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
            assert declared_tool_only["provider"] == "unknown"
            assert declared_tool_only["model"] == "gpt-4.1"
            assert declared_tool_only["method"] == "POST"
            assert declared_tool_only["status_code"] == 200
            assert declared_tool_only["messages_count"] == 1
            assert declared_tool_only["tools_count"] == 0
            assert declared_tool_only["input_tokens"] == 26
            assert declared_tool_only["output_tokens"] == 52
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

            google_stream_rows = _eventually(
                lambda: conn.execute(
                    """
                    SELECT *
                    FROM model_calls
                    WHERE path = '/v1beta/models/gemini-2.5-flash:streamGenerateContent'
                    ORDER BY id
                    """
                ).fetchall(),
                lambda rows: len(rows) >= 1,
            )
            google_stream = google_stream_rows[-1]
            _assert_event_id(google_stream["event_id"])
            assert google_stream["provider"] == "google"
            assert google_stream["model"] == "gemini-2.5-flash"
            assert google_stream["method"] == "POST"
            assert google_stream["status_code"] == 200
            assert google_stream["messages_count"] == 1
            assert google_stream["tools_count"] == 0
            assert google_stream["input_tokens"] == 5
            assert google_stream["output_tokens"] == 3
            assert google_stream["text_content"] == "Hello world!"
            assert google_stream["stop_reason"] == "end_turn"
            assert google_stream["request_bytes"] > 0
            assert google_stream["response_bytes"] > 100
            assert google_stream["credential_ref"] is not None
            _assert_credential_ref(google_stream["credential_ref"])
            assert "capsem_test_google_stream_key" not in (
                google_stream["request_body_preview"] or ""
            )

            anthropic_stream_rows = _eventually(
                lambda: conn.execute(
                    """
                    SELECT *
                    FROM model_calls
                    WHERE path = '/v1/messages'
                    ORDER BY id
                    """
                ).fetchall(),
                lambda rows: len(rows) >= 1,
            )
            anthropic_stream = anthropic_stream_rows[-1]
            _assert_event_id(anthropic_stream["event_id"])
            assert anthropic_stream["provider"] == "anthropic"
            assert anthropic_stream["model"] == "claude-sonnet-4-20250514"
            assert anthropic_stream["method"] == "POST"
            assert anthropic_stream["status_code"] == 200
            assert anthropic_stream["messages_count"] == 1
            assert anthropic_stream["tools_count"] == 0
            assert anthropic_stream["input_tokens"] == 25
            assert anthropic_stream["output_tokens"] == 5
            assert anthropic_stream["text_content"] == "Hello world!"
            assert anthropic_stream["stop_reason"] == "end_turn"
            assert anthropic_stream["request_bytes"] > 0
            assert anthropic_stream["response_bytes"] > 100
            assert anthropic_stream["credential_ref"] is not None
            _assert_credential_ref(anthropic_stream["credential_ref"])
            assert "capsem_test_anthropic_stream_key" not in (
                anthropic_stream["request_body_preview"] or ""
            )

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
            assert len(tool_rows) == sum(1 for row in model_rows if row["tools_count"] == 1) + 1
            assert {row["call_id"] for row in tool_rows} == {"call_fm3e3d2f"}
            assert {row["model_call_id"] for row in tool_rows} == {
                *(row["id"] for row in model_rows if row["tools_count"] == 1),
                unknown_shape["id"],
            }
            assert declared_tool_only["id"] not in {row["model_call_id"] for row in tool_rows}
            valid_tool_credential_refs = {
                credential_ref,
                unknown_shape["credential_ref"],
            }
            for row in tool_rows:
                _assert_event_id(row["event_id"])
                expected_provider = (
                    "unknown" if row["model_call_id"] == unknown_shape["id"] else "openai"
                )
                assert row["provider"] == expected_provider
                assert row["status"] == "observed"
                assert row["call_index"] == 0
                assert row["arguments"] == '{"query":"Capsem ironbank poem"}'
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
            assert sum(1 for row in observed_mcp_rows if row["method"] == "tools/call") == 1
            assert all(row["tool_name"] is None for row in observed_mcp_rows if row["method"] != "tools/call")
            observed_tool_call = next(
                row for row in observed_mcp_rows if row["method"] == "tools/call"
            )
            _assert_event_id(observed_tool_call["event_id"])
            assert observed_tool_call["tool_name"] == "fixture_lookup"
            assert observed_tool_call["decision"] == "allowed"
            assert observed_tool_call["trace_id"] in {row["trace_id"] for row in tool_rows}
            assert observed_tool_call["tool_name"] in {row["tool_name"] for row in tool_rows}
            assert observed_tool_call["bytes_sent"] > 0
            assert observed_tool_call["bytes_received"] > 0
            assert "fixture_lookup" in (observed_tool_call["request_preview"] or "")
            observed_tool_request = json.loads(observed_tool_call["request_preview"])
            assert observed_tool_request["jsonrpc"] == "2.0"
            assert observed_tool_request["method"] == "tools/call"
            assert observed_tool_request["params"]["name"] == "fixture_lookup"
            assert observed_tool_request["params"]["arguments"] == {
                "query": "capsem"
            }
            assert "capsem-mock-server:mcp:fixture_lookup" in (
                observed_tool_call["response_preview"] or ""
            )
            observed_tool_response = json.loads(observed_tool_call["response_preview"])
            assert observed_tool_response["result"]["content"][0]["text"] == (
                "capsem-mock-server:mcp:fixture_lookup"
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
                    SELECT event_id FROM model_calls WHERE path = '/v1beta/models/gemini-2.5-flash:streamGenerateContent'
                    UNION
                    SELECT event_id FROM model_calls WHERE path = '/v1/messages'
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
                "profiles.rules.default_unknown_model_provider",
                "profiles.rules.default_model",
            } <= {item["rule_id"] for item in shape_security_rows}
            assert any(
                item["rule_id"] == "profiles.rules.default_unknown_model_provider"
                and item["detection_level"] == "informational"
                for item in shape_security_rows
            )
            declared_tool_security_rows = security_by_event[declared_tool_only["event_id"]]
            assert {item["rule_action"] for item in declared_tool_security_rows} == {"allow"}
            assert {
                "profiles.rules.default_unknown_model_provider",
                "profiles.rules.default_model",
            } <= {item["rule_id"] for item in declared_tool_security_rows}
            for stream_model in (google_stream, anthropic_stream):
                stream_security_rows = security_by_event[stream_model["event_id"]]
                assert {item["rule_action"] for item in stream_security_rows} == {"allow"}
                assert "profiles.rules.default_model" in {
                    item["rule_id"] for item in stream_security_rows
                }
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

            model_id_before_real_clients = conn.execute(
                "SELECT COALESCE(MAX(id), 0) FROM model_calls"
            ).fetchone()[0]
            fs_id_before_real_clients = conn.execute(
                "SELECT COALESCE(MAX(id), 0) FROM fs_events"
            ).fetchone()[0]
            security_id_before_real_clients = conn.execute(
                "SELECT COALESCE(MAX(id), 0) FROM security_rule_events"
            ).fetchone()[0]
            real_client_script_name = f"ironbank-real-clients-{uuid.uuid4().hex[:8]}.py"
            real_client_script = _real_client_diversity_probe_script(mock_base_url).encode()
            real_client_upload = client.post_bytes(
                f"/vms/{session_id}/files/content?path={real_client_script_name}",
                real_client_script,
                timeout=30,
            )
            assert real_client_upload is not None
            assert real_client_upload["success"] is True
            assert real_client_upload["size"] == len(real_client_script)
            real_client_exec = client.post(
                f"/vms/{session_id}/exec",
                {"command": f"python3 /root/{real_client_script_name}", "timeout_secs": 180},
                timeout=210,
            )
            assert real_client_exec is not None, "real-client exec returned no body"
            assert real_client_exec["exit_code"] == 0, real_client_exec
            real_client_output = (real_client_exec.get("stdout") or "") + (
                real_client_exec.get("stderr") or ""
            )
            assert "capsem_test_anthropic_sdk_key" not in real_client_output
            assert "capsem_test_litellm_sdk_key" not in real_client_output
            assert "capsem_test_ollama_sdk_key" not in real_client_output
            real_client_line = next(
                (
                    line
                    for line in real_client_output.splitlines()
                    if line.startswith("IRONBANK_REAL_CLIENT_RESULT=")
                ),
                None,
            )
            assert real_client_line is not None, real_client_output
            real_client_result = json.loads(real_client_line.split("=", 1)[1])
            assert real_client_result == {
                "anthropic_model": "claude-sonnet-4-20250514",
                "anthropic_text": EXPECTED_POEM,
                "anthropic_usage_total": 30,
                "litellm_model": "gemma4:latest",
                "litellm_text": EXPECTED_POEM,
                "litellm_usage_total": 78,
                "ollama_eval_count": 5,
                "ollama_model": "gemma4:latest",
                "ollama_prompt_eval_count": 7,
                "ollama_text": EXPECTED_POEM,
                "poem_paths": {
                    "anthropic": "/root/anthropic-sdk-poem.md",
                    "litellm": "/root/litellm-poem.md",
                    "ollama": "/root/ollama-sdk-poem.md",
                },
            }
            for poem_path in real_client_result["poem_paths"].values():
                poem_status, poem_bytes = client.get_bytes(
                    f"/vms/{session_id}/files/content?path={Path(poem_path).name}",
                    timeout=30,
                )
                assert poem_status == 200
                assert poem_bytes.decode() == EXPECTED_POEM + "\n"

            real_client_models = _eventually(
                lambda: conn.execute(
                    """
                    SELECT *
                    FROM model_calls
                    WHERE id > ?
                    ORDER BY id
                    """,
                    (model_id_before_real_clients,),
                ).fetchall(),
                lambda rows: len(rows) >= 3,
            )
            by_path = {row["path"]: row for row in real_client_models}
            assert {"/v1/messages", "/v1/chat/completions", "/api/chat"} <= set(by_path)
            anthropic_sdk_row = by_path["/v1/messages"]
            assert anthropic_sdk_row["provider"] == "anthropic"
            assert anthropic_sdk_row["model"] == "claude-sonnet-4-20250514"
            assert anthropic_sdk_row["messages_count"] == 1
            assert anthropic_sdk_row["tools_count"] == 0
            assert anthropic_sdk_row["input_tokens"] == 25
            assert anthropic_sdk_row["output_tokens"] == 5
            assert anthropic_sdk_row["text_content"] == EXPECTED_POEM
            assert anthropic_sdk_row["stop_reason"] == "end_turn"
            assert anthropic_sdk_row["credential_ref"] is not None
            _assert_credential_ref(anthropic_sdk_row["credential_ref"])
            assert "capsem_test_anthropic_sdk_key" not in (
                anthropic_sdk_row["request_body_preview"] or ""
            )
            litellm_row = by_path["/v1/chat/completions"]
            assert litellm_row["provider"] == "openai"
            assert litellm_row["model"] == "gemma4:latest"
            assert litellm_row["messages_count"] == 1
            assert litellm_row["tools_count"] == 0
            assert litellm_row["input_tokens"] == 26
            assert litellm_row["output_tokens"] == 52
            assert litellm_row["text_content"] == EXPECTED_POEM
            assert litellm_row["stop_reason"] == "end_turn"
            assert litellm_row["credential_ref"] is not None
            _assert_credential_ref(litellm_row["credential_ref"])
            assert "capsem_test_litellm_sdk_key" not in (
                litellm_row["request_body_preview"] or ""
            )
            ollama_row = by_path["/api/chat"]
            assert ollama_row["provider"] == "ollama"
            assert ollama_row["model"] == "gemma4:latest"
            assert ollama_row["messages_count"] == 1
            assert ollama_row["tools_count"] == 0
            assert ollama_row["input_tokens"] == 7
            assert ollama_row["output_tokens"] == 5
            assert ollama_row["text_content"] == EXPECTED_POEM
            assert ollama_row["stop_reason"] == "end_turn"
            assert ollama_row["credential_ref"] is None

            real_client_file_rows = _eventually(
                lambda: conn.execute(
                    """
                    SELECT *
                    FROM fs_events
                    WHERE id > ?
                    ORDER BY id
                    """,
                    (fs_id_before_real_clients,),
                ).fetchall(),
                lambda rows: {
                    "anthropic-sdk-poem.md",
                    "litellm-poem.md",
                    "ollama-sdk-poem.md",
                }
                <= {Path(row["path"]).name for row in rows},
            )
            real_client_file_names = {Path(row["path"]).name for row in real_client_file_rows}
            assert {
                "anthropic-sdk-poem.md",
                "litellm-poem.md",
                "ollama-sdk-poem.md",
            } <= real_client_file_names

            real_client_security_rows = _eventually(
                lambda: conn.execute(
                    """
                    SELECT *
                    FROM security_rule_events
                    WHERE id > ?
                    ORDER BY id
                    """,
                    (security_id_before_real_clients,),
                ).fetchall(),
                lambda rows: {row["event_id"] for row in rows}
                >= {row["event_id"] for row in real_client_models},
            )
            security_by_real_client_event: dict[str, list[sqlite3.Row]] = {}
            for row in real_client_security_rows:
                security_by_real_client_event.setdefault(row["event_id"], []).append(row)
            for row in real_client_models:
                rows = security_by_real_client_event[row["event_id"]]
                assert rows
                assert all(json.loads(item["rule_json"]) for item in rows)
                assert all(json.loads(item["event_json"]) for item in rows)
                assert "allow" in {item["rule_action"] for item in rows}
                assert "profiles.rules.default_model" in {item["rule_id"] for item in rows}

            public_net_rows = conn.execute(
                """
                SELECT id, event_id, domain, port, method, path, status_code
                FROM net_events
                WHERE domain IS NOT NULL AND domain != '127.0.0.1'
                ORDER BY id
                """
            ).fetchall()
            assert public_net_rows == []
            public_dns_rows = conn.execute(
                """
                SELECT id, event_id, qname, qtype, qclass, rcode, decision
                FROM dns_events
                WHERE qname NOT LIKE ?
                ORDER BY id
                """,
                (f"{session_id}%",),
            ).fetchall()
            assert public_dns_rows == []

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


def test_codex_cli_poem_path_pays_full_ledger_debt_blackbox():
    assert MOCK_SERVER_BINARY.exists(), f"{MOCK_SERVER_BINARY} missing; restore mock server runtime"
    assert ASSETS_DIR.exists(), f"{ASSETS_DIR} missing; build VM assets before Ironbank"
    assert PROFILES_DIR.exists(), f"{PROFILES_DIR} missing; materialize profile config before Ironbank"

    service = ServiceInstance()
    client = None
    mock_proc = None
    session_id = vm_name("ironbank-codex")
    script_name = f"ironbank-codex-cli-{uuid.uuid4().hex[:8]}.py"
    try:
        service.start()
        client = service.client()
        mock_proc, ready = start_mock_server(
            request_log=service.tmp_dir / "mock-server-requests.jsonl"
        )
        mock_base_url = ready["base_url"]
        mock_request_log = Path(ready["request_log"])
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

        script = _codex_cli_probe_script(mock_base_url).encode()
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
            {"command": f"python3 /root/{script_name}", "timeout_secs": 240},
            timeout=270,
        )
        assert exec_resp is not None
        assert exec_resp["exit_code"] == 0, exec_resp
        output = (exec_resp.get("stdout") or "") + (exec_resp.get("stderr") or "")
        assert "capsem_test_codex_cli_key" not in output
        result_line = next(
            (
                line
                for line in output.splitlines()
                if line.startswith("IRONBANK_CODEX_CLI_RESULT=")
            ),
            None,
        )
        assert result_line is not None, output
        result = json.loads(result_line.split("=", 1)[1])
        nonce = result["nonce"]
        filename = result["filename"]
        assert re.fullmatch(r"[0-9a-f]{32}", nonce), result
        assert re.fullmatch(r"codex-cli-[0-9a-f]{32}\.txt", filename), result
        assert result["contains_nonce"] is True
        assert result["file_contains_nonce"] is True
        assert result["output_bytes"] > len(nonce)
        assert result["poem_bytes"] == len((nonce + "\n").encode())
        assert result["poem_path"] == f"/root/{filename}"
        assert result["broker_echo"]["has_authorization"] is True
        assert result["broker_echo"]["authorization_is_broker_ref"] is False

        poem_status, poem_bytes = client.get_bytes(
            f"/vms/{session_id}/files/content?path={filename}",
            timeout=30,
        )
        assert poem_status == 200
        assert poem_bytes.decode() == nonce + "\n"

        mock_records = [json.loads(line) for line in mock_request_log.read_text().splitlines()]
        echo_records = [row for row in mock_records if row["path"] == "/echo"]
        assert len(echo_records) >= 1
        broker_echo_record = echo_records[0]
        assert broker_echo_record["method"] == "POST"
        assert broker_echo_record["status"] == 200
        assert broker_echo_record["request_body"] == "codex broker probe"
        assert RAW_CODEX_BROKER_SECRET not in broker_echo_record["request_body"]
        responses_records = [row for row in mock_records if row["path"] == "/v1/responses"]
        assert len(responses_records) == 2
        tool_http_record, final_http_record = responses_records
        assert tool_http_record["method"] == "POST"
        assert tool_http_record["status"] == 200
        assert tool_http_record["content_type"] == "text/event-stream"
        assert tool_http_record["request_bytes"] == len(
            tool_http_record["request_body"].encode()
        )
        assert tool_http_record["response_bytes"] == len(
            tool_http_record["response_body"].encode()
        )
        tool_http_request = json.loads(tool_http_record["request_body"])
        assert tool_http_request["model"] == "gemma4:latest"
        assert tool_http_request["stream"] is True
        assert len(tool_http_request["tools"]) == 14
        assert any(tool["name"] == "exec_command" for tool in tool_http_request["tools"])
        assert nonce in tool_http_record["request_body"]
        assert f"/root/{filename}" in tool_http_record["request_body"]
        assert "call_codex_write_poem" in tool_http_record["response_body"]
        assert "response.function_call_arguments.delta" in tool_http_record["response_body"]
        assert nonce in tool_http_record["response_body"]
        assert f"/root/{filename}" in tool_http_record["response_body"]
        assert "capsem_test_codex_cli_key" not in tool_http_record["request_body"]
        assert RAW_CODEX_BROKER_SECRET not in tool_http_record["request_body"]

        assert final_http_record["method"] == "POST"
        assert final_http_record["status"] == 200
        assert final_http_record["content_type"] == "text/event-stream"
        assert final_http_record["request_bytes"] == len(
            final_http_record["request_body"].encode()
        )
        assert final_http_record["response_bytes"] == len(
            final_http_record["response_body"].encode()
        )
        final_http_request = json.loads(final_http_record["request_body"])
        assert final_http_request["model"] == "gemma4:latest"
        assert final_http_request["stream"] is True
        assert len(final_http_request["tools"]) == 14
        final_inputs = final_http_request["input"]
        assert final_inputs[-2]["type"] == "function_call"
        assert final_inputs[-2]["name"] == "exec_command"
        assert final_inputs[-2]["call_id"] == "call_codex_write_poem"
        assert nonce in final_inputs[-2]["arguments"]
        assert f"/root/{filename}" in final_inputs[-2]["arguments"]
        assert final_inputs[-1]["type"] == "function_call_output"
        assert final_inputs[-1]["call_id"] == "call_codex_write_poem"
        assert "Process exited with code 0" in final_inputs[-1]["output"]
        assert nonce not in final_inputs[-1]["output"]
        final_sse_events = [
            json.loads(line.removeprefix("data: "))
            for line in final_http_record["response_body"].splitlines()
            if line.startswith("data: ")
        ]
        assert any(event.get("delta") == nonce for event in final_sse_events)
        assert any(event.get("text") == nonce for event in final_sse_events)
        assert any(
            event.get("type") == "response.reasoning_summary_text.delta"
            and event.get("delta") == "ledger reasoning"
            for event in final_sse_events
        )
        assert "capsem_test_codex_cli_key" not in final_http_record["request_body"]
        assert RAW_CODEX_BROKER_SECRET not in final_http_record["request_body"]

        conn = _connect_session_db(service, session_id)
        try:
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
                timeout_s=30,
            )
            broker_echo_net = echo_rows[-1]
            _assert_event_id(broker_echo_net["event_id"])
            assert broker_echo_net["method"] == "POST"
            assert broker_echo_net["domain"] == "127.0.0.1"
            assert broker_echo_net["port"] == 3713
            assert broker_echo_net["status_code"] == 200
            assert broker_echo_net["decision"] == "allowed"
            _assert_credential_ref(broker_echo_net["credential_ref"])
            assert "host: 127.0.0.1:3713" in (broker_echo_net["request_headers"] or "")
            assert "authorization: hash:" in (broker_echo_net["request_headers"] or "")
            assert "content-type: text/plain" in (broker_echo_net["request_headers"] or "")
            assert broker_echo_net["request_body_preview"] is None
            assert '"authorization_is_broker_ref":false' in (
                broker_echo_net["response_body_preview"] or ""
            )
            assert '"body_size":18' in (broker_echo_net["response_body_preview"] or "")
            assert RAW_CODEX_BROKER_SECRET not in (broker_echo_net["request_headers"] or "")
            assert RAW_CODEX_BROKER_SECRET not in (broker_echo_net["request_body_preview"] or "")

            model_rows = _eventually(
                lambda: conn.execute(
                    """
                    SELECT *
                    FROM model_calls
                    WHERE path = '/v1/responses'
                    ORDER BY id
                    """
                ).fetchall(),
                lambda rows: len(rows) >= 2,
                timeout_s=30,
            )
            tool_model = model_rows[-2]
            codex_model = model_rows[-1]
            _assert_event_id(tool_model["event_id"])
            assert tool_model["provider"] == "openai"
            assert tool_model["model"] == "gemma4:latest"
            assert tool_model["method"] == "POST"
            assert tool_model["status_code"] == 200
            assert tool_model["messages_count"] >= 1
            assert tool_model["tools_count"] == 1
            assert tool_model["input_tokens"] == 31
            assert tool_model["output_tokens"] == 17
            assert tool_model["text_content"] is None
            assert tool_model["thinking_content"] is None
            assert tool_model["stop_reason"] == "end_turn"
            assert tool_model["request_bytes"] > 0
            assert tool_model["response_bytes"] > 0
            assert tool_model["credential_ref"] is None
            assert '"name":"exec_command"' in (tool_model["request_body_preview"] or "")
            assert "capsem_test_codex_cli_key" not in (
                tool_model["request_body_preview"] or ""
            )
            _assert_event_id(codex_model["event_id"])
            assert codex_model["provider"] == "openai"
            assert codex_model["model"] == "gemma4:latest"
            assert codex_model["method"] == "POST"
            assert codex_model["status_code"] == 200
            assert codex_model["messages_count"] >= 1
            assert codex_model["tools_count"] == 0
            assert codex_model["input_tokens"] == 7
            assert codex_model["output_tokens"] == 5
            assert codex_model["text_content"] == nonce
            assert codex_model["thinking_content"] == "ledger reasoning"
            assert codex_model["stop_reason"] == "end_turn"
            assert codex_model["request_bytes"] > 0
            assert codex_model["response_bytes"] > 0
            assert codex_model["credential_ref"] is None
            usage_details = json.loads(codex_model["usage_details"])
            assert usage_details["thinking"] == 2
            assert "call_codex_write_poem" in (codex_model["request_body_preview"] or "")
            assert "capsem_test_codex_cli_key" not in (
                codex_model["request_body_preview"] or ""
            )

            tool_rows = _eventually(
                lambda: conn.execute(
                    """
                    SELECT tool_calls.*, model_calls.trace_id AS model_trace_id
                    FROM tool_calls
                    JOIN model_calls ON model_calls.id = tool_calls.model_call_id
                    WHERE tool_calls.call_id = 'call_codex_write_poem'
                    ORDER BY tool_calls.id
                    """
                ).fetchall(),
                lambda rows: len(rows) == 1,
            )
            tool_row = tool_rows[0]
            _assert_event_id(tool_row["event_id"])
            assert tool_row["model_call_id"] == tool_model["id"]
            assert tool_row["provider"] == "openai"
            assert tool_row["status"] == "observed"
            assert tool_row["call_index"] == 0
            assert tool_row["tool_name"] == "exec_command"
            tool_args = json.loads(tool_row["arguments"])
            assert tool_args["cmd"] == (
                f"printf '%s\\n' {nonce} > /root/{filename}"
            )
            assert f"/root/{filename}" in tool_args["cmd"]
            assert tool_args["yield_time_ms"] == 1000
            assert tool_args["max_output_tokens"] == 2000
            assert tool_row["origin"] == "native"
            assert tool_row["trace_id"] == tool_row["model_trace_id"]
            assert tool_row["credential_ref"] is None

            tool_response_rows = _eventually(
                lambda: conn.execute(
                    """
                    SELECT *
                    FROM tool_responses
                    WHERE call_id = 'call_codex_write_poem'
                    ORDER BY id
                    """
                ).fetchall(),
                lambda rows: len(rows) == 1,
            )
            tool_response = tool_response_rows[0]
            assert tool_response["model_call_id"] == codex_model["id"]
            assert tool_response["call_id"] == "call_codex_write_poem"
            assert tool_response["is_error"] == 0
            assert tool_response["trace_id"] == codex_model["trace_id"]
            assert "Process exited with code 0" in (
                tool_response["content_preview"] or ""
            )
            assert nonce not in (tool_response["content_preview"] or "")

            net_rows = _eventually(
                lambda: conn.execute(
                    """
                    SELECT *
                    FROM net_events
                    WHERE path = '/v1/responses'
                    ORDER BY id
                    """
                ).fetchall(),
                lambda rows: len(rows) >= 2,
            )
            tool_net = net_rows[-2]
            codex_net = net_rows[-1]
            _assert_event_id(tool_net["event_id"])
            assert tool_net["method"] == "POST"
            assert tool_net["domain"] == "127.0.0.1"
            assert tool_net["port"] == 3713
            assert tool_net["status_code"] == 200
            assert tool_net["decision"] == "allowed"
            assert tool_net["credential_ref"] is None
            assert "host: 127.0.0.1:3713" in (tool_net["request_headers"] or "")
            assert "authorization:" not in (tool_net["request_headers"] or "").lower()
            assert "content-type: application/json" in (tool_net["request_headers"] or "")
            assert "user-agent:" in (tool_net["request_headers"] or "")
            assert "capsem_test_codex_cli_key" not in (tool_net["request_headers"] or "")
            assert "content-type: text/event-stream" in (
                tool_net["response_headers"] or ""
            )
            assert '"name":"exec_command"' in (tool_net["request_body_preview"] or "")
            assert "call_codex_write_poem" in (tool_net["response_body_preview"] or "")
            assert "response.function_call_arguments.delta" in (
                tool_net["response_body_preview"] or ""
            )
            _assert_event_id(codex_net["event_id"])
            assert codex_net["method"] == "POST"
            assert codex_net["domain"] == "127.0.0.1"
            assert codex_net["port"] == 3713
            assert codex_net["status_code"] == 200
            assert codex_net["decision"] == "allowed"
            assert codex_net["credential_ref"] is None
            assert codex_net["bytes_sent"] > 0
            assert codex_net["bytes_received"] > 0
            assert "host: 127.0.0.1:3713" in (codex_net["request_headers"] or "")
            assert "authorization:" not in (codex_net["request_headers"] or "").lower()
            assert "content-type: application/json" in (codex_net["request_headers"] or "")
            assert "user-agent:" in (codex_net["request_headers"] or "")
            assert "capsem_test_codex_cli_key" not in (codex_net["request_headers"] or "")
            assert "capsem_test_codex_cli_key" not in (
                codex_net["request_body_preview"] or ""
            )
            assert "call_codex_write_poem" in (codex_net["request_body_preview"] or "")
            assert "response.reasoning_summary_text.delta" in (
                codex_net["response_body_preview"] or ""
            )
            assert "response.output_text.delta" in (codex_net["response_body_preview"] or "")
            assert nonce in (codex_net["response_body_preview"] or "")

            security_rows = _eventually(
                lambda: conn.execute(
                    """
                    SELECT *
                    FROM security_rule_events
                    WHERE event_id IN (?, ?, ?, ?)
                    ORDER BY id
                    """,
                    (
                        tool_net["event_id"],
                        codex_net["event_id"],
                        tool_model["event_id"],
                        codex_model["event_id"],
                    ),
                ).fetchall(),
                lambda rows: len(rows) >= 8,
            )
            by_event: dict[str, list[sqlite3.Row]] = {}
            for row in security_rows:
                by_event.setdefault(row["event_id"], []).append(row)
                assert json.loads(row["rule_json"])
                assert json.loads(row["event_json"])
            assert "profiles.rules.default_model" in {
                row["rule_id"] for row in by_event[codex_model["event_id"]]
            }
            assert "profiles.rules.default_model" in {
                row["rule_id"] for row in by_event[tool_model["event_id"]]
            }
            assert "profiles.rules.ai_openai_model_api" in {
                row["rule_id"] for row in by_event[codex_model["event_id"]]
            }
            assert "profiles.rules.ai_openai_model_api" in {
                row["rule_id"] for row in by_event[tool_model["event_id"]]
            }
            assert "profiles.rules.default_http" in {
                row["rule_id"] for row in by_event[codex_net["event_id"]]
            }
            assert "profiles.rules.default_http" in {
                row["rule_id"] for row in by_event[tool_net["event_id"]]
            }
            assert "allow" in {row["rule_action"] for row in security_rows}
            echo_security_rows = _eventually(
                lambda: conn.execute(
                    """
                    SELECT *
                    FROM security_rule_events
                    WHERE event_id = ?
                    ORDER BY id
                    """,
                    (broker_echo_net["event_id"],),
                ).fetchall(),
                lambda rows: len(rows) >= 1,
            )
            assert "profiles.rules.default_http" in {
                row["rule_id"] for row in echo_security_rows
            }
            assert "allow" in {row["rule_action"] for row in echo_security_rows}

            public_net_rows = conn.execute(
                """
                SELECT *
                FROM net_events
                WHERE domain IS NOT NULL AND domain != '127.0.0.1'
                ORDER BY id
                """
            ).fetchall()
            assert public_net_rows == []
            public_dns_rows = conn.execute(
                """
                SELECT id, event_id, qname, qtype, qclass, rcode, answer_ip, decision
                FROM dns_events
                WHERE qname NOT LIKE ?
                ORDER BY id
                """,
                (f"{session_id}%",),
            ).fetchall()
            assert public_dns_rows == []
            session_dns_rows = conn.execute(
                """
                SELECT *
                FROM dns_events
                WHERE qname LIKE ?
                ORDER BY id
                """,
                (f"{session_id}%",),
            ).fetchall()
            assert session_dns_rows
            for row in session_dns_rows:
                _assert_event_id(row["event_id"])
                assert row["qtype"] in {1, 28}
                assert row["qclass"] == 1
                assert row["rcode"] in {0, 3}
                assert row["decision"] == "allowed"
                assert row["source_proto"] in {"udp", "tcp"}
                if row["rcode"] == 0:
                    assert row["answer_ip"] is not None
                    assert re.fullmatch(
                        r"([0-9]{1,3}\.){3}[0-9]{1,3}|[0-9a-f:]+",
                        row["answer_ip"],
                    )
                else:
                    assert row["answer_ip"] is None

            substitutions = _eventually(
                lambda: conn.execute(
                    """
                    SELECT *
                    FROM substitution_events
                    WHERE substitution_ref = ?
                    ORDER BY id
                    """,
                    (broker_echo_net["credential_ref"],),
                ).fetchall(),
                lambda rows: {row["outcome"] for row in rows} >= {"captured", "brokered"},
            )
            substitution_outcomes = {row["outcome"] for row in substitutions}
            assert {"captured", "brokered"} <= substitution_outcomes
            for row in substitutions:
                _assert_event_id(row["event_id"])
                assert row["material_class"] == "credential"
                assert row["source"] == "http.header.authorization"
                assert row["event_type"] == "http.request"
                assert row["algorithm"] == "blake3"
                assert row["substitution_ref"] == broker_echo_net["credential_ref"]
                assert row["provider"] == "openai"
                assert row["confidence"] is None
                assert row["trace_id"] == broker_echo_net["trace_id"]
                context = json.loads(row["context_json"])
                assert context["domain"] == "127.0.0.1"
                assert context["header"] == "authorization"

            substitution_security_rows = conn.execute(
                """
                SELECT *
                FROM security_rule_events
                WHERE event_id IN (
                    SELECT event_id
                    FROM substitution_events
                    WHERE substitution_ref = ?
                )
                ORDER BY id
                """,
                (broker_echo_net["credential_ref"],),
            ).fetchall()
            assert substitution_security_rows == []

            file_rows = _eventually(
                lambda: conn.execute(
                    "SELECT * FROM fs_events WHERE path = ? ORDER BY id",
                    (filename,),
                ).fetchall(),
                lambda rows: any(row["action"] in {"created", "modified"} for row in rows),
            )
            assert all(row["credential_ref"] is None for row in file_rows)
            created_file_rows = [
                row for row in file_rows if row["action"] in {"created", "modified"}
            ]
            assert all(row["directory"] == "." for row in created_file_rows)
            assert all(row["name"] == filename for row in created_file_rows)
            assert any(
                row["size"] == len((nonce + "\n").encode())
                and row["trace_id"] == tool_row["trace_id"]
                for row in created_file_rows
            )

            exec_row = conn.execute(
                "SELECT * FROM exec_events WHERE command = ? ORDER BY id DESC LIMIT 1",
                (f"python3 /root/{script_name}",),
            ).fetchone()
            assert exec_row is not None
            _assert_event_id(exec_row["event_id"])
            assert exec_row["source"] == "api"
            assert exec_row["exit_code"] == 0
            assert "IRONBANK_CODEX_CLI_RESULT" in (exec_row["stdout_preview"] or "")
            assert "capsem_test_codex_cli_key" not in (exec_row["stdout_preview"] or "")
            assert exec_row["command"] == f"python3 /root/{script_name}"
            assert exec_row["credential_ref"] is None

            audit_rows = _eventually(
                lambda: conn.execute(
                    """
                    SELECT *
                    FROM audit_events
                    WHERE argv LIKE '%codex%' OR exe LIKE '%codex%' OR comm LIKE '%codex%'
                    ORDER BY id
                    """
                ).fetchall(),
                lambda rows: len(rows) >= 1,
            )
            for row in audit_rows:
                _assert_event_id(row["event_id"])
                assert row["uid"] == 0
                assert row["exe"] or row["comm"] or row["argv"]
                assert row["credential_ref"] is None
            assert any("codex" in (row["argv"] or "") for row in audit_rows)

            security_decision_rows = conn.execute(
                """
                SELECT *
                FROM security_decision_events
                WHERE event_id IN (?, ?, ?, ?)
                ORDER BY id
                """,
                (
                    tool_net["event_id"],
                    codex_net["event_id"],
                    tool_model["event_id"],
                    codex_model["event_id"],
                ),
            ).fetchall()
            assert security_decision_rows
            for row in security_decision_rows:
                _assert_event_id(row["event_id"])
                assert row["requested_decision"] in {"allow", "ask", "block"}
                assert row["effective_decision"] in {"allow", "ask", "block"}
                assert row["stage"] in {
                    "preprocess",
                    "rule",
                    "rewrite",
                    "postprocess",
                    "ask_resolution",
                }
                if row["event_type"] == "model.call":
                    assert row["previous_decision"] == "allow"
                    assert row["requested_decision"] == "allow"
                    assert row["effective_decision"] == "allow"
                elif row["rule_id"] == "profiles.rules.ai_ollama_http_local_host":
                    assert row["previous_decision"] == "allow"
                    assert row["requested_decision"] == "allow"
                    assert row["effective_decision"] == "allow"
                elif row["rule_id"] == "profiles.rules.default_000_local_network":
                    assert row["previous_decision"] == "allow"
                    assert row["requested_decision"] == "ask"
                    assert row["effective_decision"] == "ask"
                elif row["rule_id"] == "profiles.rules.default_http":
                    assert row["previous_decision"] == "ask"
                    assert row["requested_decision"] == "allow"
                    assert row["effective_decision"] == "ask"
                assert json.loads(row["event_json"])
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
