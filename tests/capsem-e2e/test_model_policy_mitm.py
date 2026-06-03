"""Model Policy V2 MITM E2E tests."""

import base64
import json
import shlex
import sqlite3
import threading
import time
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import pytest

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB
from helpers.service import ServiceInstance, select_editable_profile, wait_exec_ready

pytestmark = pytest.mark.e2e


def _guest_python(script: str) -> str:
    encoded = base64.b64encode(script.encode()).decode()
    command = f"import base64; exec(base64.b64decode({encoded!r}).decode())"
    return f"python3 -c {shlex.quote(command)}"


def _start_service(extra_env=None) -> ServiceInstance:
    svc = ServiceInstance(extra_env=extra_env)
    svc.start()
    select_editable_profile(svc.client(), prefix="model-policy")
    return svc


def _openai_allow_rules() -> dict:
    return {
        "policy.dns.allow_e2e_openai_api": {
            "on": "dns.request",
            "if": 'qname == "api.openai.com"',
            "decision": "allow",
            "priority": 900,
            "reason": "E2E allow OpenAI DNS",
        },
        "policy.http.allow_e2e_openai_api": {
            "on": "http.request",
            "if": 'request.host == "api.openai.com"',
            "decision": "allow",
            "priority": 900,
            "reason": "E2E allow OpenAI HTTP",
        },
        "policy.model.allow_e2e_openai_requests": {
            "on": "model.request",
            "if": 'provider == "openai"',
            "decision": "allow",
            "priority": 900,
            "reason": "E2E allow OpenAI model requests",
        },
    }


def _create_vm(svc: ServiceInstance, prefix: str) -> str:
    vm = f"{prefix}-{uuid.uuid4().hex[:8]}"
    svc.client().post(
        "/provision",
        {
            "name": vm,
            "ram_mb": DEFAULT_RAM_MB,
            "cpus": DEFAULT_CPUS,
            "persistent": False,
        },
        timeout=120,
    )
    if not wait_exec_ready(svc.client(), vm):
        pytest.fail(f"VM {vm} never became exec-ready")
    return vm


def _delete_vm(svc: ServiceInstance, vm: str) -> None:
    try:
        svc.client().delete(f"/delete/{vm}", timeout=60)
    except Exception:
        pass


def _session_db(svc: ServiceInstance, vm: str) -> Path:
    return svc.tmp_dir / "sessions" / vm / "session.db"


def _wait_for_row(db_path: Path, sql: str, predicate, timeout: float = 20.0):
    deadline = time.time() + timeout
    last_rows = []
    while time.time() < deadline:
        if db_path.exists():
            conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
            conn.row_factory = sqlite3.Row
            try:
                last_rows = conn.execute(sql).fetchall()
                for row in last_rows:
                    if predicate(row):
                        return row
            finally:
                conn.close()
        time.sleep(0.2)
    pytest.fail(f"timed out waiting for row; rows={[dict(row) for row in last_rows]}")


class _OpenAiFixtureHandler(BaseHTTPRequestHandler):
    response_factory = staticmethod(lambda _body: {})
    seen_bodies = []

    def do_POST(self):
        length = int(self.headers.get("content-length", "0") or "0")
        body = self.rfile.read(length)
        body_text = body.decode(errors="replace")
        type(self).seen_bodies.append(body_text)
        response_body = json.dumps(type(self).response_factory(body_text)).encode()
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(response_body)))
        self.end_headers()
        self.wfile.write(response_body)

    def log_message(self, format, *args):
        return


class _OpenAiFixtureServer:
    def __init__(self, response_factory):
        if isinstance(response_factory, dict):
            response = response_factory
            response_factory = lambda _body: response

        factory = response_factory

        class Handler(_OpenAiFixtureHandler):
            response_factory = staticmethod(factory)
            seen_bodies = []

        self.handler = Handler
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)

    @property
    def port(self) -> int:
        return self.server.server_address[1]

    @property
    def seen_bodies(self):
        return self.handler.seen_bodies

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, exc_type, exc, tb):
        self.server.shutdown()
        self.thread.join(timeout=5)
        self.server.server_close()


def test_guest_model_request_policy_block_records_session_db_no_leak():
    svc = _start_service()
    vm = None
    try:
        saved = svc.client().post(
            "/settings",
            {
                **_openai_allow_rules(),
                "policy.model.block_e2e_openai": {
                    "on": "model.request",
                    "if": (
                        'provider == "openai" && model == "gpt-4o-mini" '
                        '&& request.body.contains("e2e-model-secret")'
                    ),
                    "decision": "block",
                    "priority": 10,
                    "reason": "E2E model policy block",
                },
            },
            timeout=30,
        )
        assert saved is not None
        assert "error" not in saved, saved
        assert (
            saved["effective_rules"]["model"]["block_e2e_openai"]["decision"] == "block"
        ), saved["effective_rules"]

        vm = _create_vm(svc, "model-policy")
        db_path = _session_db(svc, vm)
        body = {
            "model": "gpt-4o-mini",
            "messages": [
                {"role": "user", "content": "please keep e2e-model-secret local"}
            ],
        }
        script = f"""
import json
import subprocess

body = {json.dumps(json.dumps(body))}
proc = subprocess.run(
    [
        "curl",
        "-k",
        "-sS",
        "--max-time",
        "20",
        "-X",
        "POST",
        "-H",
        "content-type: application/json",
        "--data",
        body,
        "-w",
        "\\nHTTP_STATUS:%{{http_code}}",
        "https://api.openai.com/v1/chat/completions",
    ],
    capture_output=True,
    text=True,
    timeout=30,
)
print(json.dumps({{"returncode": proc.returncode, "stdout": proc.stdout, "stderr": proc.stderr}}))
"""
        response = svc.client().post(
            f"/exec/{vm}",
            {"command": _guest_python(script), "timeout_secs": 60},
            timeout=75,
        )
        assert response is not None
        assert response.get("exit_code") == 0, response
        payload = json.loads(response["stdout"].strip().splitlines()[-1])
        assert payload["returncode"] == 0, payload
        assert "HTTP_STATUS:403" in payload["stdout"], payload
        assert "policy.model.block_e2e_openai" in payload["stdout"], payload

        net_row = _wait_for_row(
            db_path,
            """
            SELECT decision, status_code, bytes_sent, policy_mode, policy_action,
                   policy_rule, policy_reason, request_body_preview
            FROM net_events
            ORDER BY id DESC
            """,
            lambda row: row["policy_rule"] == "policy.model.block_e2e_openai",
        )
        assert net_row["decision"] == "denied"
        assert net_row["status_code"] == 403
        assert net_row["bytes_sent"] > 0
        assert net_row["policy_mode"] == "runtime"
        assert net_row["policy_action"] == "block"
        assert net_row["policy_reason"] == "E2E model policy block"
        assert "e2e-model-secret" not in (net_row["request_body_preview"] or "")

        model_row = _wait_for_row(
            db_path,
            """
            SELECT provider, model, request_bytes, request_body_preview
            FROM model_calls
            ORDER BY id DESC
            """,
            lambda row: row["provider"] == "openai",
        )
        assert model_row["model"] is None
        assert model_row["request_bytes"] > 0
        assert "e2e-model-secret" not in (model_row["request_body_preview"] or "")
    finally:
        if vm is not None:
            _delete_vm(svc, vm)
        svc.stop()


def test_guest_model_request_policy_ask_allows_and_rewrite_no_leak():
    upstream = _OpenAiFixtureServer(
        lambda body_text: (
            {"ok": True, "body": body_text}
            if "[redacted-model-secret]" in body_text
            else {"ok": True}
        )
    )
    upstream.__enter__()
    svc = _start_service(
        {
            "CAPSEM_TEST_UPSTREAM_OVERRIDES": (
                f"api.openai.com:443=http://127.0.0.1:{upstream.port}"
            )
        }
    )
    vm = None
    try:
        saved = svc.client().post(
            "/settings",
            {
                **_openai_allow_rules(),
                "policy.model.ask_e2e_openai": {
                    "on": "model.request",
                    "if": (
                        'provider == "openai" && model == "gpt-4o-mini" '
                        '&& request.body.contains("ask-model-secret")'
                    ),
                    "decision": "ask",
                    "priority": 10,
                    "reason": "E2E model policy ask",
                },
                "policy.model.rewrite_e2e_openai": {
                    "on": "model.request",
                    "if": (
                        'provider == "openai" && model == "gpt-4o-mini" '
                        '&& request.body.contains("rewrite-model-secret")'
                    ),
                    "decision": "rewrite",
                    "priority": 20,
                    "reason": "E2E model request rewrite",
                    "rewrite_target": 'request.body =~ "rewrite-model-secret"',
                    "rewrite_value": "[redacted-model-secret]",
                },
            },
            timeout=30,
        )
        assert saved["effective_rules"]["model"]["ask_e2e_openai"]["decision"] == "ask"
        assert saved["effective_rules"]["model"]["rewrite_e2e_openai"]["decision"] == "rewrite"

        vm = _create_vm(svc, "model-policy-ask")
        db_path = _session_db(svc, vm)
        ask_body = {
            "model": "gpt-4o-mini",
            "messages": [
                {"role": "user", "content": "please approve ask-model-secret"}
            ],
        }
        rewrite_body = {
            "model": "gpt-4o-mini",
            "messages": [
                {"role": "user", "content": "please rewrite rewrite-model-secret"}
            ],
        }
        script = f"""
import json
import subprocess

def post(body):
    proc = subprocess.run(
        [
            "curl",
            "-k",
            "-sS",
            "--max-time",
            "20",
            "-X",
            "POST",
            "-H",
            "content-type: application/json",
            "--data",
            json.dumps(body),
            "-w",
            "\\nHTTP_STATUS:%{{http_code}}",
            "https://api.openai.com/v1/chat/completions",
        ],
        capture_output=True,
        text=True,
        timeout=30,
    )
    return {{"returncode": proc.returncode, "stdout": proc.stdout, "stderr": proc.stderr}}

print(json.dumps({{
    "ask": post({json.dumps(ask_body)}),
    "rewrite": post({json.dumps(rewrite_body)}),
}}))
"""
        response = svc.client().post(
            f"/exec/{vm}",
            {"command": _guest_python(script), "timeout_secs": 90},
            timeout=105,
        )
        assert response is not None
        assert response.get("exit_code") == 0, response
        payload = json.loads(response["stdout"].strip().splitlines()[-1])

        assert payload["ask"]["returncode"] == 0, payload
        assert "HTTP_STATUS:200" in payload["ask"]["stdout"], payload
        assert "ask-model-secret" not in payload["ask"]["stdout"], payload

        assert payload["rewrite"]["returncode"] == 0, payload
        assert "HTTP_STATUS:200" in payload["rewrite"]["stdout"], payload
        assert "[redacted-model-secret]" in payload["rewrite"]["stdout"], payload
        assert "rewrite-model-secret" not in payload["rewrite"]["stdout"], payload
        assert len(upstream.seen_bodies) == 2
        assert "ask-model-secret" in upstream.seen_bodies[0]
        assert "[redacted-model-secret]" in upstream.seen_bodies[1]
        assert "rewrite-model-secret" not in upstream.seen_bodies[1]

        ask_row = _wait_for_row(
            db_path,
            """
            SELECT decision, status_code, bytes_sent, policy_mode, policy_action,
                   policy_rule, policy_reason, request_body_preview
            FROM net_events
            ORDER BY id DESC
            """,
            lambda row: row["policy_rule"] == "policy.model.ask_e2e_openai",
        )
        assert ask_row["decision"] == "allowed"
        assert ask_row["status_code"] == 200
        assert ask_row["bytes_sent"] > 0
        assert ask_row["policy_mode"] == "runtime"
        assert ask_row["policy_action"] == "allow"
        assert ask_row["policy_reason"] == "E2E model policy ask"
        assert "ask-model-secret" in (ask_row["request_body_preview"] or "")

        rewrite_row = _wait_for_row(
            db_path,
            """
            SELECT decision, status_code, bytes_sent, policy_mode, policy_action,
                   policy_rule, policy_reason, request_body_preview
            FROM net_events
            ORDER BY id DESC
            """,
            lambda row: row["policy_rule"] == "policy.model.rewrite_e2e_openai",
        )
        assert rewrite_row["decision"] == "allowed"
        assert rewrite_row["status_code"] == 200
        assert rewrite_row["bytes_sent"] > 0
        assert rewrite_row["policy_mode"] == "runtime"
        assert rewrite_row["policy_action"] == "rewrite"
        assert rewrite_row["policy_reason"] == "E2E model request rewrite"
        assert "[redacted-model-secret]" in (
            rewrite_row["request_body_preview"] or ""
        )
        assert "rewrite-model-secret" not in (
            rewrite_row["request_body_preview"] or ""
        )
    finally:
        if vm is not None:
            _delete_vm(svc, vm)
        svc.stop()
        upstream.__exit__(None, None, None)


def test_guest_model_tool_response_policy_block_and_rewrite_no_leak():
    svc = _start_service()
    vm = None
    try:
        saved = svc.client().post(
            "/settings",
            {
                **_openai_allow_rules(),
                "policy.model.block_e2e_tool_response": {
                    "on": "model.tool_response",
                    "if": (
                        'provider == "openai" && model == "gpt-4o-mini" '
                        '&& tool.call_id == "call_block" '
                        '&& content.contains("tool-block-secret")'
                    ),
                    "decision": "block",
                    "priority": 10,
                    "reason": "E2E block secret tool output",
                },
                "policy.model.rewrite_e2e_tool_response": {
                    "on": "model.tool_response",
                    "if": (
                        'provider == "openai" && model == "gpt-4o-mini" '
                        '&& tool.call_id == "call_rewrite" '
                        '&& content.contains("tool-rewrite-secret")'
                    ),
                    "decision": "rewrite",
                    "priority": 20,
                    "reason": "E2E redact secret tool output",
                    "rewrite_target": 'content =~ "tool-rewrite-secret"',
                    "rewrite_value": "[redacted-tool-secret]",
                },
            },
            timeout=30,
        )
        assert saved["effective_rules"]["model"]["block_e2e_tool_response"]["decision"] == "block"
        assert (
            saved["effective_rules"]["model"]["rewrite_e2e_tool_response"]["decision"]
            == "rewrite"
        )

        vm = _create_vm(svc, "model-tool-policy")
        db_path = _session_db(svc, vm)
        block_body = {
            "model": "gpt-4o-mini",
            "messages": [
                {"role": "user", "content": "lookup secret"},
                {
                    "role": "assistant",
                    "tool_calls": [
                        {
                            "id": "call_block",
                            "type": "function",
                            "function": {"name": "lookup", "arguments": "{}"},
                        }
                    ],
                },
                {
                    "role": "tool",
                    "tool_call_id": "call_block",
                    "content": "local output tool-block-secret",
                },
            ],
        }
        rewrite_body = {
            "model": "gpt-4o-mini",
            "messages": [
                {"role": "user", "content": "lookup secret"},
                {
                    "role": "assistant",
                    "tool_calls": [
                        {
                            "id": "call_rewrite",
                            "type": "function",
                            "function": {"name": "lookup", "arguments": "{}"},
                        }
                    ],
                },
                {
                    "role": "tool",
                    "tool_call_id": "call_rewrite",
                    "content": "local output tool-rewrite-secret",
                },
            ],
        }
        script = f"""
import json
import subprocess

def post(body):
    proc = subprocess.run(
        [
            "curl",
            "-k",
            "-sS",
            "--max-time",
            "20",
            "-X",
            "POST",
            "-H",
            "content-type: application/json",
            "--data",
            json.dumps(body),
            "-w",
            "\\nHTTP_STATUS:%{{http_code}}",
            "https://api.openai.com/v1/chat/completions",
        ],
        capture_output=True,
        text=True,
        timeout=30,
    )
    return {{"returncode": proc.returncode, "stdout": proc.stdout, "stderr": proc.stderr}}

print(json.dumps({{
    "block": post({json.dumps(block_body)}),
    "rewrite": post({json.dumps(rewrite_body)}),
}}))
"""
        response = svc.client().post(
            f"/exec/{vm}",
            {"command": _guest_python(script), "timeout_secs": 90},
            timeout=105,
        )
        assert response is not None
        assert response.get("exit_code") == 0, response
        payload = json.loads(response["stdout"].strip().splitlines()[-1])

        assert payload["block"]["returncode"] == 0, payload
        assert "HTTP_STATUS:403" in payload["block"]["stdout"], payload
        assert "policy.model.block_e2e_tool_response" in payload["block"][
            "stdout"
        ], payload
        assert "tool-block-secret" not in payload["block"]["stdout"], payload

        assert payload["rewrite"]["returncode"] == 0, payload
        assert "tool-rewrite-secret" not in json.dumps(payload["rewrite"]), payload

        block_row = _wait_for_row(
            db_path,
            """
            SELECT decision, status_code, policy_mode, policy_action,
                   policy_rule, policy_reason, request_body_preview
            FROM net_events
            ORDER BY id DESC
            """,
            lambda row: row["policy_rule"] == "policy.model.block_e2e_tool_response",
        )
        assert block_row["decision"] == "denied"
        assert block_row["status_code"] == 403
        assert block_row["policy_mode"] == "runtime"
        assert block_row["policy_action"] == "block"
        assert block_row["policy_reason"] == "E2E block secret tool output"
        assert "tool-block-secret" not in (block_row["request_body_preview"] or "")

        rewrite_row = _wait_for_row(
            db_path,
            """
            SELECT decision, status_code, policy_mode, policy_action,
                   policy_rule, policy_reason, request_body_preview
            FROM net_events
            ORDER BY id DESC
            """,
            lambda row: row["policy_rule"] == "policy.model.rewrite_e2e_tool_response",
            timeout=30.0,
        )
        assert rewrite_row["policy_mode"] == "runtime"
        assert rewrite_row["policy_action"] == "rewrite"
        assert rewrite_row["policy_reason"] == "E2E redact secret tool output"
        assert "[redacted-tool-secret]" in (
            rewrite_row["request_body_preview"] or ""
        )
        assert "tool-rewrite-secret" not in (
            rewrite_row["request_body_preview"] or ""
        )

        tool_response_row = _wait_for_row(
            db_path,
            """
            SELECT tr.call_id, tr.content_preview
            FROM tool_responses tr
            JOIN model_calls mc ON mc.id = tr.model_call_id
            ORDER BY tr.id DESC
            """,
            lambda row: row["call_id"] == "call_rewrite",
            timeout=30.0,
        )
        assert "[redacted-tool-secret]" in (
            tool_response_row["content_preview"] or ""
        )
        assert "tool-rewrite-secret" not in (
            tool_response_row["content_preview"] or ""
        )
    finally:
        if vm is not None:
            _delete_vm(svc, vm)
        svc.stop()


def test_guest_model_response_and_tool_call_policy_with_fixture_upstream_no_leak():
    def response_for(body_text: str) -> dict:
        if "response-policy-case" in body_text:
            return {
                "id": "chatcmpl-response-policy",
                "object": "chat.completion",
                "model": "gpt-4o-mini",
                "choices": [
                    {
                        "index": 0,
                        "message": {
                            "role": "assistant",
                            "content": "fixture says e2e-response-secret",
                        },
                        "finish_reason": "stop",
                    }
                ],
                "usage": {
                    "prompt_tokens": 1,
                    "completion_tokens": 1,
                    "total_tokens": 2,
                },
            }
        if "response-rewrite-case" in body_text:
            return {
                "id": "chatcmpl-response-rewrite-policy",
                "object": "chat.completion",
                "model": "gpt-4o-mini",
                "choices": [
                    {
                        "index": 0,
                        "message": {
                            "role": "assistant",
                            "content": "fixture says e2e-response-rewrite-secret",
                        },
                        "finish_reason": "stop",
                    }
                ],
                "usage": {
                    "prompt_tokens": 1,
                    "completion_tokens": 1,
                    "total_tokens": 2,
                },
            }
        if "tool-call-block-case" in body_text:
            return {
                "id": "chatcmpl-tool-block-policy",
                "object": "chat.completion",
                "model": "gpt-4o-mini",
                "choices": [
                    {
                        "index": 0,
                        "message": {
                            "role": "assistant",
                            "content": None,
                            "tool_calls": [
                                {
                                    "id": "call_tool_block_policy",
                                    "type": "function",
                                    "function": {
                                        "name": "search",
                                        "arguments": json.dumps(
                                            {"query": "tool-call-block-secret"}
                                        ),
                                    },
                                }
                            ],
                        },
                        "finish_reason": "tool_calls",
                    }
                ],
                "usage": {
                    "prompt_tokens": 1,
                    "completion_tokens": 1,
                    "total_tokens": 2,
                },
            }
        return {
            "id": "chatcmpl-tool-policy",
            "object": "chat.completion",
            "model": "gpt-4o-mini",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": None,
                        "tool_calls": [
                            {
                                "id": "call_tool_policy",
                                "type": "function",
                                "function": {
                                    "name": "search",
                                    "arguments": json.dumps(
                                        {"query": "tool-call-secret"}
                                    ),
                                },
                            }
                        ],
                    },
                    "finish_reason": "tool_calls",
                }
            ],
            "usage": {
                "prompt_tokens": 1,
                "completion_tokens": 1,
                "total_tokens": 2,
            },
        }

    with _OpenAiFixtureServer(response_for) as upstream:
        svc = _start_service(
            {
                "CAPSEM_TEST_UPSTREAM_OVERRIDES": (
                    f"api.openai.com:443=http://127.0.0.1:{upstream.port}"
                )
            }
        )
        vm = None
        try:
            saved = svc.client().post(
                "/settings",
                {
                    **_openai_allow_rules(),
                    "policy.model.block_e2e_model_response": {
                        "on": "model.response",
                        "if": (
                            'provider == "openai" && model == "gpt-4o-mini" '
                            '&& response.text.contains("e2e-response-secret")'
                        ),
                        "decision": "block",
                        "priority": 10,
                        "reason": "E2E response secret block",
                    },
                    "policy.model.rewrite_e2e_model_response": {
                        "on": "model.response",
                        "if": (
                            'provider == "openai" && model == "gpt-4o-mini" '
                            '&& response.text.contains("e2e-response-rewrite-secret")'
                        ),
                        "decision": "rewrite",
                        "priority": 15,
                        "reason": "E2E response secret rewrite",
                        "rewrite_target": (
                            'response.text =~ "e2e-response-rewrite-secret"'
                        ),
                        "rewrite_value": "[redacted-response]",
                    },
                    "policy.model.block_e2e_tool_call": {
                        "on": "model.tool_call",
                        "if": (
                            'provider == "openai" && model == "gpt-4o-mini" '
                            '&& tool.name == "search" '
                            '&& tool.arguments.query == "tool-call-block-secret"'
                        ),
                        "decision": "block",
                        "priority": 16,
                        "reason": "E2E block model tool call",
                    },
                    "policy.model.rewrite_e2e_tool_call": {
                        "on": "model.tool_call",
                        "if": (
                            'provider == "openai" && model == "gpt-4o-mini" '
                            '&& tool.name == "search" '
                            '&& tool.arguments.query == "tool-call-secret"'
                        ),
                        "decision": "rewrite",
                        "priority": 20,
                        "reason": "E2E redact model tool call",
                        "rewrite_target": 'tool.arguments.query =~ "tool-call-secret"',
                        "rewrite_value": "[redacted-query]",
                    },
                },
                timeout=30,
            )
            assert (
                saved["effective_rules"]["model"]["block_e2e_model_response"]["decision"]
                == "block"
            )
            assert (
                saved["effective_rules"]["model"]["rewrite_e2e_tool_call"]["decision"]
                == "rewrite"
            )
            assert (
                saved["effective_rules"]["model"]["rewrite_e2e_model_response"]["decision"]
                == "rewrite"
            )
            assert (
                saved["effective_rules"]["model"]["block_e2e_tool_call"]["decision"]
                == "block"
            )

            vm = _create_vm(svc, "model-response-policy")
            db_path = _session_db(svc, vm)
            response_body = {
                "model": "gpt-4o-mini",
                "messages": [
                    {"role": "user", "content": "response-policy-case"}
                ],
            }
            rewrite_response_body = {
                "model": "gpt-4o-mini",
                "messages": [
                    {"role": "user", "content": "response-rewrite-case"}
                ],
            }
            block_tool_body = {
                "model": "gpt-4o-mini",
                "messages": [
                    {"role": "user", "content": "tool-call-block-case"}
                ],
            }
            tool_body = {
                "model": "gpt-4o-mini",
                "messages": [
                    {"role": "user", "content": "tool-call-policy-case"}
                ],
            }
            script = f"""
import json
import subprocess

def post(body):
    proc = subprocess.run(
        [
            "curl",
            "-k",
            "-sS",
            "--max-time",
            "20",
            "--resolve",
            "api.openai.com:443:198.18.0.1",
            "-X",
            "POST",
            "-H",
            "content-type: application/json",
            "--data",
            json.dumps(body),
            "-w",
            "\\nHTTP_STATUS:%{{http_code}}",
            "https://api.openai.com/v1/chat/completions",
        ],
        capture_output=True,
        text=True,
        timeout=30,
    )
    return {{"returncode": proc.returncode, "stdout": proc.stdout, "stderr": proc.stderr}}

print(json.dumps({{
    "response": post({json.dumps(response_body)}),
    "response_rewrite": post({json.dumps(rewrite_response_body)}),
    "tool_block": post({json.dumps(block_tool_body)}),
    "tool": post({json.dumps(tool_body)}),
}}))
"""
            response = svc.client().post(
                f"/exec/{vm}",
                {"command": _guest_python(script), "timeout_secs": 90},
                timeout=105,
            )
            assert response is not None
            assert response.get("exit_code") == 0, response
            payload = json.loads(response["stdout"].strip().splitlines()[-1])

            assert payload["response"]["returncode"] == 0, payload
            assert "HTTP_STATUS:403" in payload["response"]["stdout"], payload
            assert "policy.model.block_e2e_model_response" in payload["response"][
                "stdout"
            ], payload
            assert "e2e-response-secret" not in payload["response"]["stdout"], payload

            assert payload["response_rewrite"]["returncode"] == 0, payload
            assert "HTTP_STATUS:200" in payload["response_rewrite"]["stdout"], payload
            assert "[redacted-response]" in payload["response_rewrite"]["stdout"], payload
            assert "e2e-response-rewrite-secret" not in payload["response_rewrite"][
                "stdout"
            ], payload

            assert payload["tool_block"]["returncode"] == 0, payload
            assert "HTTP_STATUS:403" in payload["tool_block"]["stdout"], payload
            assert "policy.model.block_e2e_tool_call" in payload["tool_block"][
                "stdout"
            ], payload
            assert "tool-call-block-secret" not in payload["tool_block"]["stdout"], payload

            assert payload["tool"]["returncode"] == 0, payload
            assert "HTTP_STATUS:200" in payload["tool"]["stdout"], payload
            assert "[redacted-query]" in payload["tool"]["stdout"], payload
            assert "tool-call-secret" not in payload["tool"]["stdout"], payload

            assert any("response-policy-case" in body for body in upstream.seen_bodies)
            assert any("response-rewrite-case" in body for body in upstream.seen_bodies)
            assert any("tool-call-block-case" in body for body in upstream.seen_bodies)
            assert any("tool-call-policy-case" in body for body in upstream.seen_bodies)

            response_row = _wait_for_row(
                db_path,
                """
                SELECT decision, status_code, policy_mode, policy_action,
                       policy_rule, policy_reason, response_body_preview
                FROM net_events
                ORDER BY id DESC
                """,
                lambda row: row["policy_rule"]
                == "policy.model.block_e2e_model_response",
                timeout=30.0,
            )
            assert response_row["decision"] == "denied"
            assert response_row["status_code"] == 403
            assert response_row["policy_action"] == "block"
            assert response_row["policy_reason"] == "E2E response secret block"
            assert "e2e-response-secret" not in (
                response_row["response_body_preview"] or ""
            )

            response_security_row = _wait_for_row(
                db_path,
                """
                SELECT se.event_id, se.event_family, se.event_type,
                       se.source_engine, se.final_action, se.enforceability,
                       se.origin_kind, se.trace_id, step.kind, step.status,
                       step.rule_id
                FROM security_events se
                JOIN security_event_steps step ON step.event_id = se.event_id
                WHERE se.event_type = 'model.response'
                ORDER BY se.id DESC, step.step_index ASC
                """,
                lambda row: row["rule_id"]
                == "policy.model.block_e2e_model_response",
                timeout=30.0,
            )
            assert response_security_row["event_family"] == "model"
            assert response_security_row["event_type"] == "model.response"
            assert response_security_row["source_engine"] == "network"
            assert response_security_row["final_action"] == "block"
            assert response_security_row["enforceability"] == "inline_blockable"
            assert response_security_row["origin_kind"] == "guest_network"
            assert response_security_row["kind"] == "enforcement_match"
            assert response_security_row["status"] == "matched"
            assert response_security_row["trace_id"]

            response_evidence_row = _wait_for_row(
                db_path,
                """
                SELECT provider, api_family, model, parse_status,
                       evidence_status, response_text_preview, trace_id
                FROM ai_model_interactions
                ORDER BY id DESC
                """,
                lambda row: row["trace_id"] == response_security_row["trace_id"]
                and "e2e-response-secret" in (row["response_text_preview"] or ""),
                timeout=30.0,
            )
            assert response_evidence_row["provider"] == "openai"
            assert response_evidence_row["api_family"] == "openai_chat_completions"
            assert response_evidence_row["model"] == "gpt-4o-mini"
            assert response_evidence_row["parse_status"] == "complete"
            assert response_evidence_row["evidence_status"] == "complete"

            hunt = svc.client().post(
                f"/sessions/{vm}/detection/hunt",
                {
                    "rules": [
                        {
                            "id": "detect-e2e-canonical-model-response",
                            "pack_id": "e2e-security-event",
                            "title": "Detect E2E canonical model response",
                            "condition": (
                                "common.event_type == 'model.response' "
                                "&& model.response.provider == 'openai' "
                                "&& model.response.body.text.contains('e2e-response-secret')"
                            ),
                            "severity": "high",
                            "confidence": "high",
                            "tags": ["e2e", "security-event"],
                        }
                    ],
                    "limit": 20,
                },
                timeout=30,
            )
            assert hunt["total_matches"] >= 1, hunt
            assert any(
                row["event_ref"]["event_id"] == response_security_row["event_id"]
                and row["rule_id"] == "detect-e2e-canonical-model-response"
                and any(
                    field["path"] == "model.response.body.text"
                    and "e2e-response-secret" in json.dumps(field["value"])
                    for field in row["matched_fields"]
                )
                for row in hunt["rows"]
            ), hunt

            response_rewrite_row = _wait_for_row(
                db_path,
                """
                SELECT decision, status_code, policy_mode, policy_action,
                       policy_rule, policy_reason, response_body_preview
                FROM net_events
                ORDER BY id DESC
                """,
                lambda row: row["policy_rule"]
                == "policy.model.rewrite_e2e_model_response",
                timeout=30.0,
            )
            assert response_rewrite_row["decision"] == "allowed"
            assert response_rewrite_row["status_code"] == 200
            assert response_rewrite_row["policy_action"] == "rewrite"
            assert response_rewrite_row["policy_reason"] == "E2E response secret rewrite"
            assert "[redacted-response]" in (
                response_rewrite_row["response_body_preview"] or ""
            )
            assert "e2e-response-rewrite-secret" not in (
                response_rewrite_row["response_body_preview"] or ""
            )

            tool_block_row = _wait_for_row(
                db_path,
                """
                SELECT decision, status_code, policy_mode, policy_action,
                       policy_rule, policy_reason, response_body_preview
                FROM net_events
                ORDER BY id DESC
                """,
                lambda row: row["policy_rule"]
                == "policy.model.block_e2e_tool_call",
                timeout=30.0,
            )
            assert tool_block_row["decision"] == "denied"
            assert tool_block_row["status_code"] == 403
            assert tool_block_row["policy_action"] == "block"
            assert tool_block_row["policy_reason"] == "E2E block model tool call"
            assert "tool-call-block-secret" not in (
                tool_block_row["response_body_preview"] or ""
            )

            tool_row = _wait_for_row(
                db_path,
                """
                SELECT decision, status_code, policy_mode, policy_action,
                       policy_rule, policy_reason, response_body_preview
                FROM net_events
                ORDER BY id DESC
                """,
                lambda row: row["policy_rule"]
                == "policy.model.rewrite_e2e_tool_call",
                timeout=30.0,
            )
            assert tool_row["decision"] == "allowed"
            assert tool_row["status_code"] == 200
            assert tool_row["policy_action"] == "rewrite"
            assert tool_row["policy_reason"] == "E2E redact model tool call"
            assert "[redacted-query]" in (
                tool_row["response_body_preview"] or ""
            )
            assert "tool-call-secret" not in (tool_row["response_body_preview"] or "")

        finally:
            if vm is not None:
                _delete_vm(svc, vm)
            svc.stop()
