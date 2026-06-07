"""Model Policy V2 MITM E2E tests."""

import base64
import json
import shlex
import sqlite3
import time
import uuid
from pathlib import Path

import pytest

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB
from helpers.service import ServiceInstance, wait_exec_ready

pytestmark = pytest.mark.e2e


def _guest_python(script: str) -> str:
    encoded = base64.b64encode(script.encode()).decode()
    command = f"import base64; exec(base64.b64decode({encoded!r}).decode())"
    return f"python3 -c {shlex.quote(command)}"


def _start_service() -> ServiceInstance:
    svc = ServiceInstance()
    svc.start()
    return svc


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


def test_guest_model_request_policy_block_records_session_db_no_leak():
    svc = _start_service()
    vm = None
    try:
        saved = svc.client().post(
            "/settings",
            {
                "security.web.allow_write": True,
                "ai.openai.allow": True,
                "ai.openai.domains": "api.openai.com, *.openai.com",
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
            saved["policy"]["model"]["block_e2e_openai"]["decision"] == "block"
        ), saved["policy"]

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
        assert net_row["policy_mode"] == "enforce"
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


def test_guest_model_request_policy_ask_and_rewrite_fail_closed_no_leak():
    svc = _start_service()
    vm = None
    try:
        saved = svc.client().post(
            "/settings",
            {
                "security.web.allow_write": True,
                "ai.openai.allow": True,
                "ai.openai.domains": "api.openai.com, *.openai.com",
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
                    "reason": "E2E model request rewrite fail closed",
                    "rewrite_target": 'request.body =~ "rewrite-model-secret"',
                    "rewrite_value": "[redacted-model-secret]",
                },
            },
            timeout=30,
        )
        assert saved["policy"]["model"]["ask_e2e_openai"]["decision"] == "ask"
        assert saved["policy"]["model"]["rewrite_e2e_openai"]["decision"] == "rewrite"

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
        assert "HTTP_STATUS:403" in payload["ask"]["stdout"], payload
        assert "policy.model.ask_e2e_openai" in payload["ask"]["stdout"], payload
        assert "ask-model-secret" not in payload["ask"]["stdout"], payload

        assert payload["rewrite"]["returncode"] == 0, payload
        assert "HTTP_STATUS:403" in payload["rewrite"]["stdout"], payload
        assert "policy.model.rewrite_e2e_openai" in payload["rewrite"]["stdout"], payload
        assert "rewrite-model-secret" not in payload["rewrite"]["stdout"], payload

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
        assert ask_row["decision"] == "denied"
        assert ask_row["status_code"] == 403
        assert ask_row["bytes_sent"] > 0
        assert ask_row["policy_mode"] == "enforce"
        assert ask_row["policy_action"] == "ask"
        assert ask_row["policy_reason"] == "E2E model policy ask"
        assert "ask-model-secret" not in (ask_row["request_body_preview"] or "")

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
        assert rewrite_row["decision"] == "denied"
        assert rewrite_row["status_code"] == 403
        assert rewrite_row["bytes_sent"] > 0
        assert rewrite_row["policy_mode"] == "enforce"
        assert rewrite_row["policy_action"] == "rewrite"
        assert "not implemented yet" in rewrite_row["policy_reason"]
        assert "rewrite-model-secret" not in (
            rewrite_row["request_body_preview"] or ""
        )
    finally:
        if vm is not None:
            _delete_vm(svc, vm)
        svc.stop()


def test_guest_model_tool_response_policy_block_and_rewrite_no_leak():
    svc = _start_service()
    vm = None
    try:
        saved = svc.client().post(
            "/settings",
            {
                "security.web.allow_write": True,
                "ai.openai.allow": True,
                "ai.openai.domains": "api.openai.com, *.openai.com",
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
        assert saved["policy"]["model"]["block_e2e_tool_response"]["decision"] == "block"
        assert (
            saved["policy"]["model"]["rewrite_e2e_tool_response"]["decision"]
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
        assert block_row["policy_mode"] == "enforce"
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
        assert rewrite_row["policy_mode"] == "enforce"
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
