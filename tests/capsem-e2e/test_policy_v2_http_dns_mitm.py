"""Policy V2 HTTP/DNS MITM E2E tests."""

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


def _example_com_allow_rules() -> dict:
    return {
        "policy.dns.allow_e2e_example_com": {
            "on": "dns.request",
            "if": 'qname == "example.com"',
            "decision": "allow",
            "priority": 900,
            "reason": "E2E allow example.com DNS",
        },
        "policy.http.allow_e2e_example_com": {
            "on": "http.request",
            "if": 'request.host == "example.com"',
            "decision": "allow",
            "priority": 900,
            "reason": "E2E allow example.com HTTP",
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


def test_guest_http_policy_v2_block_and_header_strip_records_session_db():
    svc = _start_service()
    vm = None
    try:
        saved = svc.client().post(
            "/settings",
            {
                **_example_com_allow_rules(),
                "policy.http.block_e2e_path_query_header": {
                    "on": "http.request",
                    "if": (
                        'request.host == "example.com" && request.method == "GET" '
                        '&& request.path == "/policy-v2-block" '
                        '&& request.query == "token=secret" '
                        '&& request.headers.authorization == "Bearer http-block-secret"'
                    ),
                    "decision": "block",
                    "priority": 10,
                    "reason": "E2E HTTP path/query/header block",
                },
                "policy.http.rewrite_e2e_strip_authorization": {
                    "on": "http.request",
                    "if": (
                        'request.host == "example.com" '
                        '&& request.path == "/policy-v2-strip" '
                        "&& has(request.headers.authorization)"
                    ),
                    "decision": "rewrite",
                    "priority": 20,
                    "reason": "E2E HTTP request header strip",
                    "rewrite_target": 'request.path =~ "^/policy-v2-strip$"',
                    "rewrite_value": "/",
                    "strip_request_headers": ["Authorization"],
                },
                "policy.http.rewrite_e2e_strip_response_server": {
                    "on": "http.response",
                    "if": (
                        'request.host == "example.com" '
                        '&& request.path == "/response-strip-e2e" '
                        "&& has(response.headers.server)"
                    ),
                    "decision": "rewrite",
                    "priority": 30,
                    "reason": "E2E HTTP response header strip",
                    "strip_response_headers": ["Server"],
                },
            },
            timeout=30,
        )
        assert saved["effective_rules"]["http"]["block_e2e_path_query_header"]["decision"] == "block"
        assert (
            saved["effective_rules"]["http"]["rewrite_e2e_strip_authorization"][
                "strip_request_headers"
            ]
            == ["authorization"]
        )
        assert (
            saved["effective_rules"]["http"]["rewrite_e2e_strip_response_server"][
                "strip_response_headers"
            ]
            == ["server"]
        )

        vm = _create_vm(svc, "http-policy-v2")
        db_path = _session_db(svc, vm)
        script = r'''
import json
import subprocess

blocked = subprocess.run(
    [
        "curl",
        "-k",
        "-sS",
        "--max-time",
        "20",
        "-H",
        "Authorization: Bearer http-block-secret",
        "-w",
        "\nHTTP_STATUS:%{http_code}",
        "https://example.com/policy-v2-block?token=secret",
    ],
    capture_output=True,
    text=True,
    timeout=30,
)

stripped = subprocess.run(
    [
        "curl",
        "-k",
        "-sS",
        "--max-time",
        "20",
        "-H",
        "Authorization: Bearer http-strip-secret",
        "-w",
        "\nHTTP_STATUS:%{http_code}",
        "https://example.com/policy-v2-strip?visible=yes",
    ],
    capture_output=True,
    text=True,
    timeout=30,
)

response_stripped = subprocess.run(
    [
        "curl",
        "-k",
        "-sS",
        "--max-time",
        "20",
        "-D",
        "-",
        "-o",
        "/dev/null",
        "-w",
        "\nHTTP_STATUS:%{http_code}",
        "https://example.com/response-strip-e2e",
    ],
    capture_output=True,
    text=True,
    timeout=30,
)

print(json.dumps({
    "blocked": {
        "returncode": blocked.returncode,
        "stdout": blocked.stdout,
        "stderr": blocked.stderr,
    },
    "stripped": {
        "returncode": stripped.returncode,
        "stdout": stripped.stdout,
        "stderr": stripped.stderr,
    },
    "response_stripped": {
        "returncode": response_stripped.returncode,
        "stdout": response_stripped.stdout,
        "stderr": response_stripped.stderr,
    },
}))
'''
        response = svc.client().post(
            f"/exec/{vm}",
            {"command": _guest_python(script), "timeout_secs": 90},
            timeout=105,
        )
        assert response is not None
        assert response.get("exit_code") == 0, response
        payload = json.loads(response["stdout"].strip().splitlines()[-1])
        assert payload["blocked"]["returncode"] == 0, payload
        assert "HTTP_STATUS:403" in payload["blocked"]["stdout"], payload
        assert "policy.http.block_e2e_path_query_header" in payload["blocked"][
            "stdout"
        ], payload
        assert payload["stripped"]["returncode"] == 0, payload
        assert "http-strip-secret" not in json.dumps(payload)
        assert payload["response_stripped"]["returncode"] == 0, payload
        response_headers = payload["response_stripped"]["stdout"].lower()
        assert "server:" not in response_headers, payload
        assert "http_status:" in response_headers, payload

        block_row = _wait_for_row(
            db_path,
            """
            SELECT domain, method, path, query, decision, status_code,
                   policy_mode, policy_action, policy_rule, policy_reason,
                   request_headers, bytes_sent, bytes_received
            FROM net_events
            ORDER BY id DESC
            """,
            lambda row: row["policy_rule"] == "policy.http.block_e2e_path_query_header",
        )
        assert block_row["domain"] == "example.com"
        assert block_row["method"] == "GET"
        assert block_row["path"] == "/policy-v2-block"
        assert block_row["query"] == "token=secret"
        assert block_row["decision"] == "denied"
        assert block_row["status_code"] == 403
        assert block_row["policy_mode"] == "enforce"
        assert block_row["policy_action"] == "block"
        assert block_row["policy_reason"] == "E2E HTTP path/query/header block"
        assert block_row["bytes_sent"] == 0
        assert block_row["bytes_received"] > 0
        assert "http-block-secret" not in (block_row["request_headers"] or "")

        strip_row = _wait_for_row(
            db_path,
            """
            SELECT domain, method, path, query, decision, status_code,
                   policy_mode, policy_action, policy_rule, policy_reason,
                   request_headers, bytes_sent, bytes_received
            FROM net_events
            ORDER BY id DESC
            """,
            lambda row: row["policy_rule"]
            == "policy.http.rewrite_e2e_strip_authorization",
        )
        assert strip_row["domain"] == "example.com"
        assert strip_row["method"] == "GET"
        assert strip_row["path"] == "/"
        assert strip_row["query"] == "visible=yes"
        assert strip_row["decision"] == "allowed"
        assert strip_row["policy_mode"] == "enforce"
        assert strip_row["policy_action"] == "rewrite"
        assert strip_row["policy_reason"] == "E2E HTTP request header strip"
        assert "authorization" not in (strip_row["request_headers"] or "").lower()
        assert "http-strip-secret" not in (strip_row["request_headers"] or "")
        assert strip_row["bytes_received"] > 0

        response_strip_row = _wait_for_row(
            db_path,
            """
            SELECT domain, method, path, query, decision, status_code,
                   policy_mode, policy_action, policy_rule, policy_reason,
                   request_headers, response_headers, bytes_sent, bytes_received
            FROM net_events
            ORDER BY id DESC
            """,
            lambda row: row["policy_rule"]
            == "policy.http.rewrite_e2e_strip_response_server",
        )
        assert response_strip_row["domain"] == "example.com"
        assert response_strip_row["method"] == "GET"
        assert response_strip_row["path"] == "/response-strip-e2e"
        assert response_strip_row["decision"] == "allowed"
        assert response_strip_row["policy_mode"] == "enforce"
        assert response_strip_row["policy_action"] == "rewrite"
        assert (
            response_strip_row["policy_reason"]
            == "E2E HTTP response header strip"
        )
        assert "server:" not in (
            response_strip_row["response_headers"] or ""
        ).lower()
    finally:
        if vm is not None:
            _delete_vm(svc, vm)
        svc.stop()


def test_guest_dns_policy_v2_block_and_rewrite_records_session_db():
    svc = _start_service()
    vm = None
    try:
        saved = svc.client().post(
            "/settings",
            {
                "policy.dns.block_e2e_dns": {
                    "on": "dns.request",
                    "if": 'qname == "block-dns-e2e.capsem.test" && qtype == "A"',
                    "decision": "block",
                    "priority": 10,
                    "reason": "E2E DNS block",
                },
                "policy.dns.rewrite_e2e_dns": {
                    "on": "dns.request",
                    "if": 'qname == "rewrite-dns-e2e.capsem.test" && qtype == "A"',
                    "decision": "rewrite",
                    "priority": 20,
                    "reason": "E2E DNS rewrite",
                    "rewrite_target": 'answer.ip =~ ".*"',
                    "rewrite_value": "203.0.113.77",
                },
            },
            timeout=30,
        )
        assert saved["effective_rules"]["dns"]["block_e2e_dns"]["decision"] == "block"
        assert saved["effective_rules"]["dns"]["rewrite_e2e_dns"]["decision"] == "rewrite"

        vm = _create_vm(svc, "dns-policy-v2")
        db_path = _session_db(svc, vm)
        script = f"""
import json
import socket

def resolve_v4(name):
    try:
        infos = socket.getaddrinfo(name, None, socket.AF_INET)
        return sorted({{item[4][0] for item in infos}})
    except socket.gaierror as exc:
        return {{"error": str(exc)}}

print(json.dumps({{
    "blocked": resolve_v4("block-dns-e2e.capsem.test"),
    "rewritten": resolve_v4("rewrite-dns-e2e.capsem.test"),
}}))
"""
        response = svc.client().post(
            f"/exec/{vm}",
            {"command": _guest_python(script), "timeout_secs": 60},
            timeout=75,
        )
        assert response is not None
        assert response.get("exit_code") == 0, response
        payload = json.loads(response["stdout"].strip().splitlines()[-1])
        assert "error" in payload["blocked"], payload
        assert payload["rewritten"] == ["203.0.113.77"], payload

        block_row = _wait_for_row(
            db_path,
            """
            SELECT qname, qtype, qclass, rcode, decision, matched_rule,
                   source_proto, process_name, upstream_resolver_ms,
                   policy_mode, policy_action, policy_rule, policy_reason
            FROM dns_events
            ORDER BY id DESC
            """,
            lambda row: row["policy_rule"] == "policy.dns.block_e2e_dns",
        )
        assert block_row["qname"] == "block-dns-e2e.capsem.test"
        assert block_row["qtype"] == 1
        assert block_row["qclass"] == 1
        assert block_row["rcode"] == 3
        assert block_row["decision"] == "denied"
        assert block_row["matched_rule"] == "policy.dns.block_e2e_dns"
        assert block_row["source_proto"] == "udp"
        assert block_row["upstream_resolver_ms"] == 0
        assert block_row["policy_mode"] == "enforce"
        assert block_row["policy_action"] == "block"
        assert block_row["policy_reason"] == "E2E DNS block"

        rewrite_row = _wait_for_row(
            db_path,
            """
            SELECT qname, qtype, qclass, rcode, decision, matched_rule,
                   source_proto, process_name, upstream_resolver_ms,
                   policy_mode, policy_action, policy_rule, policy_reason
            FROM dns_events
            ORDER BY id DESC
            """,
            lambda row: row["policy_rule"] == "policy.dns.rewrite_e2e_dns",
        )
        assert rewrite_row["qname"] == "rewrite-dns-e2e.capsem.test"
        assert rewrite_row["qtype"] == 1
        assert rewrite_row["qclass"] == 1
        assert rewrite_row["rcode"] == 0
        assert rewrite_row["decision"] == "redirected"
        assert rewrite_row["matched_rule"] == "policy.dns.rewrite_e2e_dns"
        assert rewrite_row["source_proto"] == "udp"
        assert rewrite_row["upstream_resolver_ms"] == 0
        assert rewrite_row["policy_mode"] == "enforce"
        assert rewrite_row["policy_action"] == "rewrite"
        assert rewrite_row["policy_reason"] == "E2E DNS rewrite"
    finally:
        if vm is not None:
            _delete_vm(svc, vm)
        svc.stop()
