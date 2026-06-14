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
from ironbank.model_ledger import ModelLedgerRun, ModelLedgerSpec, assert_model_ledger_exchange

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


def _common_result_script_prelude(base_url: str, filename_prefix: str) -> str:
    return f"""
import json
import os
from pathlib import Path
import socket
import subprocess
import urllib.request
import uuid

BASE_URL = {json.dumps(base_url.rstrip("/"))}
DNS_QNAME = "model.capsem.test"
DNS_IP = socket.gethostbyname(DNS_QNAME)
NONCE = uuid.uuid4().hex
FILENAME = {json.dumps(filename_prefix)} + "-" + uuid.uuid4().hex + ".txt"
TARGET = "/root/" + FILENAME
PROMPT = "Write uuid4 hex value " + NONCE + " to " + TARGET + "."

def run_tool(arguments):
    command = arguments.get("cmd") or arguments.get("command")
    if command:
        completed = subprocess.run(
            command,
            shell=True,
            cwd="/root",
            capture_output=True,
            text=True,
            timeout=30,
        )
        return "Process exited with code " + str(completed.returncode)
    path = arguments.get("file_path")
    content = arguments.get("content")
    if path and content is not None:
        Path(path).write_text(content, encoding="utf-8")
        return "Process exited with code 0"
    raise RuntimeError("unsupported tool args: " + json.dumps(arguments, sort_keys=True))

def emit_result(provider, domain, path, model, output, reasoning, tool_call_name, call_args, call_response):
    file_text = Path(TARGET).read_text(encoding="utf-8")
    result = {{
        "input": PROMPT,
        "reasoning": reasoning,
        "output": output,
        "tool_call_name": tool_call_name,
        "call_args": call_args,
        "call_response": call_response,
        "provider": provider,
        "domain": domain,
        "path": path,
        "model": model,
        "target": TARGET,
        "filename": FILENAME,
        "nonce": NONCE,
        "file_text": file_text,
        "file_matches": file_text == NONCE + "\\n",
        "dns_qname": DNS_QNAME,
        "dns_ip": DNS_IP,
    }}
    print("IRONBANK_CLIENT_RESULT=" + json.dumps(result, sort_keys=True))
"""


def _openai_responses_api_script(base_url: str) -> str:
    return textwrap.dedent(
        _common_result_script_prelude(base_url, "openai-api")
        + r'''
def parse_sse(body):
    events = []
    for line in body.splitlines():
        if line.startswith("data: ") and line[6:] != "[DONE]":
            events.append(json.loads(line[6:]))
    return events

def post(body):
    req = urllib.request.Request(
        BASE_URL + "/v1/responses",
        data=json.dumps(body).encode(),
        headers={"content-type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=60) as response:
        return response.read().decode()

first_body = {
    "model": "gemma4:latest",
    "stream": True,
    "input": PROMPT,
    "tools": [{"type": "function", "name": "exec_command"}],
}
first_events = parse_sse(post(first_body))
tool_item = next(event["item"] for event in first_events if event.get("type") == "response.output_item.done")
call_args = json.loads(tool_item["arguments"])
call_response = run_tool(call_args)
second_body = {
    "model": "gemma4:latest",
    "stream": True,
    "input": [
        {"type": "function_call", "call_id": tool_item["call_id"], "name": tool_item["name"], "arguments": tool_item["arguments"]},
        {"type": "function_call_output", "call_id": tool_item["call_id"], "output": call_response},
        {"role": "user", "content": PROMPT},
    ],
    "tools": [{"type": "function", "name": "exec_command"}],
}
second_events = parse_sse(post(second_body))
output = next(event["text"] for event in second_events if event.get("type") == "response.output_text.done")
reasoning = next(event["delta"] for event in second_events if event.get("type") == "response.reasoning_summary_text.delta")
emit_result("openai", "127.0.0.1", "/v1/responses", "gemma4:latest", output, reasoning, tool_item["name"], call_args, call_response)
'''
    ).strip()


def _openai_two_tool_calls_script(base_url: str) -> str:
    return textwrap.dedent(
        _common_result_script_prelude(base_url, "openai-two")
        + r'''
def parse_sse(body):
    events = []
    for line in body.splitlines():
        if line.startswith("data: ") and line[6:] != "[DONE]":
            events.append(json.loads(line[6:]))
    return events

def post(body):
    req = urllib.request.Request(
        BASE_URL + "/v1/responses",
        data=json.dumps(body).encode(),
        headers={"content-type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=60) as response:
        return response.read().decode()

def run_one(index):
    nonce = uuid.uuid4().hex
    filename = "openai-two-" + uuid.uuid4().hex + ".txt"
    target = "/root/" + filename
    prompt = "Write uuid4 hex value " + nonce + " to " + target + "."
    first_events = parse_sse(post({
        "model": "gemma4:latest",
        "stream": True,
        "input": prompt,
        "tools": [{"type": "function", "name": "exec_command"}],
    }))
    tool_item = next(event["item"] for event in first_events if event.get("type") == "response.output_item.done")
    call_args = json.loads(tool_item["arguments"])
    call_response = run_tool(call_args)
    second_events = parse_sse(post({
        "model": "gemma4:latest",
        "stream": True,
        "input": [
            {"type": "function_call", "call_id": tool_item["call_id"], "name": tool_item["name"], "arguments": tool_item["arguments"]},
            {"type": "function_call_output", "call_id": tool_item["call_id"], "output": call_response},
            {"role": "user", "content": prompt},
        ],
        "tools": [{"type": "function", "name": "exec_command"}],
    }))
    output = next(event["text"] for event in second_events if event.get("type") == "response.output_text.done")
    reasoning = next(event["delta"] for event in second_events if event.get("type") == "response.reasoning_summary_text.delta")
    file_text = Path(target).read_text(encoding="utf-8")
    return {
        "index": index,
        "input": prompt,
        "reasoning": reasoning,
        "output": output,
        "tool_call_name": tool_item["name"],
        "call_id": tool_item["call_id"],
        "call_args": call_args,
        "call_response": call_response,
        "filename": filename,
        "target": target,
        "nonce": nonce,
        "file_matches": file_text == nonce + "\n",
    }

results = [run_one(1), run_one(2)]
print("IRONBANK_CLIENT_RESULT=" + json.dumps({
    "provider": "openai",
    "domain": "127.0.0.1",
    "path": "/v1/responses",
    "model": "gemma4:latest",
    "dns_qname": DNS_QNAME,
    "dns_ip": DNS_IP,
    "results": results,
}, sort_keys=True))
'''
    ).strip()


def _claude_api_script(base_url: str) -> str:
    return textwrap.dedent(
        _common_result_script_prelude(base_url, "claude-api")
        + r'''
def post(body):
    req = urllib.request.Request(
        BASE_URL + "/v1/messages",
        data=json.dumps(body).encode(),
        headers={"content-type": "application/json", "x-api-key": "capsem_claude_api_key_0123456789abcdef", "anthropic-version": "2023-06-01"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=60) as response:
        return json.loads(response.read().decode())

first = post({
    "model": "claude-sonnet-4-20250514",
    "max_tokens": 128,
    "messages": [{"role": "user", "content": PROMPT}],
    "tools": [{"name": "exec_command", "description": "run a command", "input_schema": {"type": "object", "properties": {"cmd": {"type": "string"}}}}],
})
tool_item = next(part for part in first["content"] if part["type"] == "tool_use")
call_args = tool_item["input"]
call_response = run_tool(call_args)
second = post({
    "model": "claude-sonnet-4-20250514",
    "max_tokens": 128,
    "messages": [
        {"role": "user", "content": PROMPT},
        {"role": "assistant", "content": [tool_item]},
        {"role": "user", "content": [{"type": "tool_result", "tool_use_id": tool_item["id"], "content": call_response}]},
    ],
    "tools": [{"name": "exec_command", "description": "run a command", "input_schema": {"type": "object", "properties": {"cmd": {"type": "string"}}}}],
})
reasoning = next(part["thinking"] for part in second["content"] if part["type"] == "thinking")
output = next(part["text"] for part in second["content"] if part["type"] == "text")
emit_result("anthropic", "127.0.0.1", "/v1/messages", "claude-sonnet-4-20250514", output, reasoning, tool_item["name"], call_args, call_response)
'''
    ).strip()


def _claude_sdk_script(base_url: str) -> str:
    return textwrap.dedent(
        _common_result_script_prelude(base_url, "claude-sdk")
        + r'''
import anthropic

client = anthropic.Anthropic(
    base_url=BASE_URL,
    api_key="capsem_claude_sdk_key_0123456789abcdef",
)
tools = [{"name": "exec_command", "description": "run a command", "input_schema": {"type": "object", "properties": {"cmd": {"type": "string"}}}}]
first = client.messages.create(
    model="claude-sonnet-4-20250514",
    max_tokens=128,
    messages=[{"role": "user", "content": PROMPT}],
    tools=tools,
)
tool_item = next(part for part in first.content if part.type == "tool_use")
call_args = dict(tool_item.input)
call_response = run_tool(call_args)
second = client.messages.create(
    model="claude-sonnet-4-20250514",
    max_tokens=128,
    messages=[
        {"role": "user", "content": PROMPT},
        {"role": "assistant", "content": [tool_item.model_dump()]},
        {"role": "user", "content": [{"type": "tool_result", "tool_use_id": tool_item.id, "content": call_response}]},
    ],
    tools=tools,
)
reasoning = next(part.thinking for part in second.content if part.type == "thinking")
output = next(part.text for part in second.content if part.type == "text")
emit_result("anthropic", "127.0.0.1", "/v1/messages", "claude-sonnet-4-20250514", output, reasoning, tool_item.name, call_args, call_response)
'''
    ).strip()


def _codex_cli_script(base_url: str) -> str:
    return textwrap.dedent(
        _common_result_script_prelude(base_url, "codex-cli")
        + r'''
codex_config = Path("/root/.codex/config.toml")
codex_text = codex_config.read_text(encoding="utf-8")
codex_text = codex_text.replace('base_url = "http://127.0.0.1:11434/v1"', 'base_url = "' + BASE_URL + '/v1"')
if "check_for_update_on_startup" not in codex_text:
    codex_text += "\ncheck_for_update_on_startup = false\n[analytics]\nenabled = false\n"
codex_config.write_text(codex_text, encoding="utf-8")
env = os.environ.copy()
env["HOME"] = "/root"
env["NO_COLOR"] = "1"
env["TERM"] = "xterm-256color"
env["OPENAI_API_KEY"] = "capsem_codex_cli_key_0123456789abcdef"
completed = subprocess.run(
    [
        "codex",
        "exec",
        "--dangerously-bypass-approvals-and-sandbox",
        "--skip-git-repo-check",
        "--cd",
        "/root",
        PROMPT,
    ],
    cwd="/root",
    env=env,
    capture_output=True,
    text=True,
    timeout=180,
)
if completed.returncode != 0:
    raise SystemExit((completed.stdout or "") + (completed.stderr or ""))
call_args = {"cmd": "printf '%s\\n' " + NONCE + " > " + TARGET, "yield_time_ms": 1000, "max_output_tokens": 2000}
emit_result("openai", "127.0.0.1", "/v1/responses", "gemma4:latest", NONCE, "ledger reasoning", "exec_command", call_args, "Process exited with code 0")
'''
    ).strip()


def _agy_cli_script(_base_url: str) -> str:
    return textwrap.dedent(
        _common_result_script_prelude("http://127.0.0.1:11434", "agy-cli")
        + r'''
env = os.environ.copy()
env["HOME"] = "/root"
env["NO_COLOR"] = "1"
env["TERM"] = "xterm-256color"
completed = subprocess.run(
    ["agy", "-p", PROMPT, "--print-timeout", "90s"],
    cwd="/root",
    env=env,
    capture_output=True,
    text=True,
    timeout=150,
)
if completed.returncode != 0:
    raise SystemExit((completed.stdout or "") + (completed.stderr or ""))
call_args = {"cmd": "printf '%s\\n' " + NONCE + " > " + TARGET, "yield_time_ms": 1000, "max_output_tokens": 2000}
emit_result("ollama", "127.0.0.1", "/api/chat", "gemma4:latest", NONCE, "ledger reasoning", "exec_command", call_args, "Process exited with code 0")
'''
    ).strip()


def _assert_one_client(env: ModelClientEnv, script: str, *, raw_secrets: tuple[str, ...] = ()) -> None:
    result = env.run_python(script)
    assert result["file_matches"] is True, result
    spec = ModelLedgerSpec(
        input=result["input"],
        reasoning=result["reasoning"],
        output=result["output"],
        tool_call_name=result["tool_call_name"],
        call_args=result["call_args"],
        call_response=result["call_response"],
        provider=result["provider"],
        domain=result["domain"],
        path=result["path"],
        model=result["model"],
    )
    run = ModelLedgerRun(
        db_path=env.db_path,
        upstream_transcript_path=env.upstream_transcript_path,
        log_paths=env.log_paths,
        raw_secrets=raw_secrets,
    )
    assert_model_ledger_exchange(spec, run)


def test_openai_responses_api_ledger_contract(model_client_env: ModelClientEnv):
    _assert_one_client(model_client_env, _openai_responses_api_script(model_client_env.mock_base_url))


def test_openai_two_tool_calls_have_exact_item_cardinality_red(
    model_client_env: ModelClientEnv,
):
    result = model_client_env.run_python(_openai_two_tool_calls_script(model_client_env.mock_base_url))
    assert len(result["results"]) == 2, result
    assert all(item["file_matches"] for item in result["results"]), result
    assert len({item["call_id"] for item in result["results"]}) == 2, result
    assert len({item["filename"] for item in result["results"]}) == 2, result

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


def test_codex_cli_ledger_contract(model_client_env: ModelClientEnv):
    _assert_one_client(
        model_client_env,
        _codex_cli_script(model_client_env.mock_base_url),
        raw_secrets=("capsem_codex_cli_key_0123456789abcdef",),
    )


def test_claude_http_api_ledger_contract(model_client_env: ModelClientEnv):
    _assert_one_client(
        model_client_env,
        _claude_api_script(model_client_env.mock_base_url),
        raw_secrets=("capsem_claude_api_key_0123456789abcdef",),
    )


def test_claude_sdk_ledger_contract(model_client_env: ModelClientEnv):
    _assert_one_client(
        model_client_env,
        _claude_sdk_script(model_client_env.mock_base_url),
        raw_secrets=("capsem_claude_sdk_key_0123456789abcdef",),
    )


def test_agy_cli_ledger_contract(model_client_env: ModelClientEnv):
    _assert_one_client(model_client_env, _agy_cli_script(model_client_env.mock_base_url))
