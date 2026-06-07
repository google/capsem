"""Guest MCP over MITM E2E tests.

These tests exercise the real guest `/run/capsem-mcp-server` bridge with
the default framed transport. Traffic goes through the MITM listener on
vsock:5002 and writes `mcp_calls` from the MITM frame layer.
"""

import base64
import json
import os
import shlex
import sqlite3
import subprocess
import sys
import textwrap
import time
import uuid
from pathlib import Path

import pytest

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB
from helpers.service import ServiceInstance, wait_exec_ready

PROJECT_ROOT = Path(__file__).parent.parent.parent
CLI_BINARY = PROJECT_ROOT / "target/debug/capsem"

pytestmark = pytest.mark.e2e


def _guest_python(script: str) -> str:
    encoded = base64.b64encode(script.encode()).decode()
    command = f"import base64; exec(base64.b64decode({encoded!r}).decode())"
    return f"python3 -c {shlex.quote(command)}"


def _start_service():
    svc = ServiceInstance()
    svc.start()
    return svc


def _create_vm(svc: ServiceInstance, prefix: str, *, persistent: bool = False) -> str:
    vm = f"{prefix}-{uuid.uuid4().hex[:8]}"
    svc.client().post(
        "/provision",
        {
            "name": vm,
            "ram_mb": DEFAULT_RAM_MB,
            "cpus": DEFAULT_CPUS,
            "persistent": persistent,
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


def _exec_cli(svc: ServiceInstance, vm: str, command: str, *, timeout: int = 120):
    return subprocess.run(
        [
            str(CLI_BINARY),
            "--uds-path",
            str(svc.uds_path),
            "exec",
            "--timeout",
            str(timeout),
            vm,
            command,
        ],
        capture_output=True,
        text=True,
        timeout=timeout + 15,
    )


def _start_cli_exec(svc: ServiceInstance, vm: str, command: str, *, timeout: int = 120):
    return subprocess.Popen(
        [
            str(CLI_BINARY),
            "--uds-path",
            str(svc.uds_path),
            "exec",
            "--timeout",
            str(timeout),
            vm,
            command,
        ],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )


def _session_db(svc: ServiceInstance, vm: str, *, persistent: bool = False) -> Path:
    kind = "persistent" if persistent else "sessions"
    return svc.tmp_dir / kind / vm / "session.db"


def _query_mcp_rows(db_path: Path):
    if not db_path.exists():
        return []
    conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    conn.row_factory = sqlite3.Row
    try:
        return conn.execute(
            """
            SELECT request_id, method, server_name, tool_name, decision,
                   process_name, policy_mode, policy_action, policy_rule,
                   policy_reason, error_message, request_preview, response_preview
            FROM mcp_calls
            ORDER BY id
            """
        ).fetchall()
    finally:
        conn.close()


def _wait_for_mcp_row(db_path: Path, predicate, timeout: float = 20.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        for row in _query_mcp_rows(db_path):
            if predicate(row):
                return row
        time.sleep(0.2)
    rows = [dict(row) for row in _query_mcp_rows(db_path)]
    pytest.fail(f"timed out waiting for mcp_calls row; rows={rows}")


def _query_net_rows(db_path: Path):
    if not db_path.exists():
        return []
    conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    conn.row_factory = sqlite3.Row
    try:
        return conn.execute(
            """
            SELECT domain, method, path, decision, process_name, status_code,
                   bytes_sent, bytes_received, conn_type
            FROM net_events
            ORDER BY id
            """
        ).fetchall()
    finally:
        conn.close()


def _wait_for_net_row(db_path: Path, predicate, timeout: float = 20.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        for row in _query_net_rows(db_path):
            if predicate(row):
                return row
        time.sleep(0.2)
    rows = [dict(row) for row in _query_net_rows(db_path)]
    pytest.fail(f"timed out waiting for net_events row; rows={rows}")


def _responses_by_id(stdout: str) -> dict[object, dict]:
    payload = json.loads(stdout.strip().splitlines()[-1])
    return {resp["id"]: resp for resp in payload["responses"] if "id" in resp}


def _guest_mcp_smoke_command(client_name: str, list_id: str) -> str:
    script = f'''
import json
import subprocess
import sys

client_name = {client_name!r}
list_id = {list_id!r}
messages = [
    {{"jsonrpc": "2.0", "id": f"{{list_id}}-init", "method": "initialize", "params": {{
        "protocolVersion": "2024-11-05",
        "capabilities": {{}},
        "clientInfo": {{"name": client_name, "version": "1.0"}},
    }}}},
    {{"jsonrpc": "2.0", "method": "notifications/initialized"}},
    {{"jsonrpc": "2.0", "id": list_id, "method": "tools/list"}},
]

proc = subprocess.run(
    ["/run/capsem-mcp-server"],
    input="\\n".join(json.dumps(m) for m in messages) + "\\n",
    capture_output=True,
    text=True,
    timeout=30,
)
responses = [json.loads(line) for line in proc.stdout.splitlines() if line.strip()]
print(json.dumps({{"returncode": proc.returncode, "stderr": proc.stderr, "responses": responses}}))
sys.exit(proc.returncode)
'''
    return _guest_python(script)


def test_framed_guest_mcp_tools_call_and_session_db_rows():
    svc = _start_service()
    vm = None
    try:
        vm = _create_vm(svc, "framed-mcp")
        script = r'''
import json
import subprocess
import sys

messages = [
    {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2024-11-05",
        "capabilities": {},
        "clientInfo": {"name": "framed-e2e", "version": "1.0"},
    }},
    {"jsonrpc": "2.0", "method": "notifications/initialized"},
    {"jsonrpc": "2.0", "id": 2, "method": "tools/list"},
    {"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {
        "name": "local__echo",
        "arguments": {"text": "framed-e2e"},
    }},
    {"jsonrpc": "2.0", "id": 4, "method": "resources/list"},
    {"jsonrpc": "2.0", "id": 5, "method": "prompts/list"},
]

proc = subprocess.run(
    ["/run/capsem-mcp-server"],
    input="\n".join(json.dumps(m) for m in messages) + "\n",
    capture_output=True,
    text=True,
    timeout=30,
)
responses = [json.loads(line) for line in proc.stdout.splitlines() if line.strip()]
print(json.dumps({
    "returncode": proc.returncode,
    "stderr": proc.stderr,
    "responses": responses,
}))
sys.exit(proc.returncode)
'''
        result = _exec_cli(svc, vm, _guest_python(script), timeout=90)
        assert result.returncode == 0, result.stderr
        responses = _responses_by_id(result.stdout)

        assert responses[1]["result"]["serverInfo"]["name"] == "capsem-mcp-mitm-endpoint"
        tool_names = {tool["name"] for tool in responses[2]["result"]["tools"]}
        assert "local__echo" in tool_names
        assert "local__fetch_http" in tool_names
        assert "framed-e2e" in json.dumps(responses[3]["result"])
        assert "resources" in responses[4]["result"]
        assert "prompts" in responses[5]["result"]

        db_path = _session_db(svc, vm)
        for request_id, method in {
            "1": "initialize",
            "2": "tools/list",
            "3": "tools/call",
            "4": "resources/list",
            "5": "prompts/list",
        }.items():
            row = _wait_for_mcp_row(
                db_path,
                lambda r, request_id=request_id: r["request_id"] == request_id,
            )
            assert row["method"] == method
            assert row["decision"] == "allowed"
            assert row["process_name"] == "python3"
            assert row["policy_mode"] == "audit_only"
            assert row["policy_action"] == "allow"

        rows = _query_mcp_rows(db_path)
        notifications = [
            row for row in rows if row["request_id"] is None and row["method"] == "notifications/initialized"
        ]
        assert len(notifications) == 1
        counts = {}
        for row in rows:
            if row["request_id"] is None:
                continue
            counts[row["request_id"]] = counts.get(row["request_id"], 0) + 1
        assert counts == {"1": 1, "2": 1, "3": 1, "4": 1, "5": 1}
        echo = [r for r in rows if r["request_id"] == "3"][0]
        assert echo["tool_name"] == "local__echo"
        assert echo["server_name"] == "local"
        assert "framed-e2e" in echo["request_preview"]
    finally:
        if vm is not None:
            _delete_vm(svc, vm)
        svc.stop()


def test_framed_guest_mcp_invalid_json_notifications_and_string_ids():
    svc = _start_service()
    vm = None
    try:
        vm = _create_vm(svc, "framed-adversarial")
        script = r'''
import json
import subprocess
import sys

lines = [
    "{not json",
    json.dumps({"jsonrpc": "2.0", "id": "init-string", "method": "initialize", "params": {
        "protocolVersion": "2024-11-05",
        "capabilities": {},
        "clientInfo": {"name": "adversarial-e2e", "version": "1.0"},
    }}),
    json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    json.dumps({"jsonrpc": "2.0", "method": "$/progress", "params": {
        "progressToken": "p1",
        "progress": 1,
        "total": 2,
    }}),
    json.dumps({"jsonrpc": "2.0", "id": "tools-list-string", "method": "tools/list"}),
]

proc = subprocess.run(
    ["/run/capsem-mcp-server"],
    input="\n".join(lines) + "\n",
    capture_output=True,
    text=True,
    timeout=30,
)
responses = [json.loads(line) for line in proc.stdout.splitlines() if line.strip()]
print(json.dumps({"returncode": proc.returncode, "stderr": proc.stderr, "responses": responses}))
sys.exit(proc.returncode)
'''
        result = _exec_cli(svc, vm, _guest_python(script), timeout=90)
        assert result.returncode == 0, result.stderr
        payload = json.loads(result.stdout.strip().splitlines()[-1])
        responses = payload["responses"]
        by_id = {resp["id"]: resp for resp in responses if "id" in resp}

        parse_errors = [
            resp
            for resp in responses
            if "id" not in resp and resp.get("error", {}).get("code") == -32700
        ]
        assert len(parse_errors) == 1, payload
        assert by_id["init-string"]["result"]["serverInfo"]["name"] == "capsem-mcp-mitm-endpoint"
        assert "local__echo" in json.dumps(by_id["tools-list-string"]["result"])
        assert len(responses) == 3, "notifications must not produce JSON-RPC responses"

        db_path = _session_db(svc, vm)
        init = _wait_for_mcp_row(db_path, lambda r: r["request_id"] == "init-string")
        tools = _wait_for_mcp_row(db_path, lambda r: r["request_id"] == "tools-list-string")
        assert init["method"] == "initialize"
        assert tools["method"] == "tools/list"
        rows = _query_mcp_rows(db_path)
        assert {row["request_id"] for row in rows if row["request_id"] is not None} == {
            "init-string",
            "tools-list-string",
        }
        notifications = [
            row for row in rows if row["request_id"] is None and row["method"] == "notifications/initialized"
        ]
        assert len(notifications) == 1
    finally:
        if vm is not None:
            _delete_vm(svc, vm)
        svc.stop()


def test_framed_guest_mcp_oversized_request_returns_local_error_and_recovers():
    svc = _start_service()
    vm = None
    try:
        vm = _create_vm(svc, "framed-oversized")
        script = r'''
import json
import subprocess
import sys

oversized = {
    "jsonrpc": "2.0",
    "id": "too-big",
    "method": "tools/call",
    "params": {
        "name": "local__echo",
        "arguments": {"text": "x" * 1100000},
    },
}
followup = {
    "jsonrpc": "2.0",
    "id": "after-oversized",
    "method": "initialize",
    "params": {
        "protocolVersion": "2024-11-05",
        "capabilities": {},
        "clientInfo": {"name": "oversized-e2e", "version": "1.0"},
    },
}

proc = subprocess.run(
    ["/run/capsem-mcp-server"],
    input=json.dumps(oversized) + "\n" + json.dumps(followup) + "\n",
    capture_output=True,
    text=True,
    timeout=30,
)
responses = [json.loads(line) for line in proc.stdout.splitlines() if line.strip()]
print(json.dumps({"returncode": proc.returncode, "stderr": proc.stderr, "responses": responses}))
sys.exit(proc.returncode)
'''
        result = _exec_cli(svc, vm, _guest_python(script), timeout=90)
        assert result.returncode == 0, result.stderr
        payload = json.loads(result.stdout.strip().splitlines()[-1])
        by_id = {resp["id"]: resp for resp in payload["responses"] if "id" in resp}
        assert by_id["too-big"]["error"]["code"] == -32001
        assert "frame encode failed" in by_id["too-big"]["error"]["message"]
        assert by_id["after-oversized"]["result"]["serverInfo"]["name"] == (
            "capsem-mcp-mitm-endpoint"
        )

        db_path = _session_db(svc, vm)
        row = _wait_for_mcp_row(db_path, lambda r: r["request_id"] == "after-oversized")
        assert row["method"] == "initialize"
        assert all(row["request_id"] != "too-big" for row in _query_mcp_rows(db_path))
    finally:
        if vm is not None:
            _delete_vm(svc, vm)
        svc.stop()


def test_framed_guest_mcp_raw_corrupt_frame_recovers_on_same_connection():
    svc = _start_service()
    vm = None
    try:
        vm = _create_vm(svc, "framed-corrupt")
        script = r'''
import json
import socket
import struct

MCP_FRAME_VERSION = 1
MCP_FRAME_HEADER_LEN = 16
MCP_FRAME_MAGIC = 0x4D43

def encode_frame(stream_id, process_name, payload, *, magic=MCP_FRAME_MAGIC):
    process_bytes = process_name.encode()
    payload_bytes = payload.encode()
    body = struct.pack(
        ">HBBIHHI",
        magic,
        MCP_FRAME_VERSION,
        MCP_FRAME_HEADER_LEN,
        stream_id,
        0,
        len(process_bytes),
        len(payload_bytes),
    ) + process_bytes + payload_bytes
    return struct.pack(">I", len(body)) + body

def read_exact(sock, length):
    chunks = []
    remaining = length
    while remaining:
        chunk = sock.recv(remaining)
        if not chunk:
            raise EOFError("socket closed")
        chunks.append(chunk)
        remaining -= len(chunk)
    return b"".join(chunks)

def read_frame(sock):
    length = struct.unpack(">I", read_exact(sock, 4))[0]
    body = read_exact(sock, length)
    stream_id = struct.unpack(">I", body[4:8])[0]
    process_len = struct.unpack(">H", body[10:12])[0]
    payload_len = struct.unpack(">I", body[12:16])[0]
    payload_start = MCP_FRAME_HEADER_LEN + process_len
    payload = body[payload_start:payload_start + payload_len]
    return {"stream_id": stream_id, "payload": json.loads(payload)}

sock = socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM)
sock.settimeout(10)
sock.connect((socket.VMADDR_CID_HOST, 5002))
sock.sendall(b"\0CAPSEM_META:raw-corruptor\n")

initial_payload = json.dumps({"jsonrpc": "2.0", "id": "before-corrupt", "method": "initialize", "params": {
    "protocolVersion": "2024-11-05",
    "capabilities": {},
    "clientInfo": {"name": "raw-corruptor", "version": "1.0"},
}})
sock.sendall(encode_frame(76, "raw-corruptor", initial_payload))
initial_response = read_frame(sock)

bad_payload = json.dumps({"jsonrpc": "2.0", "id": "bad-frame", "method": "tools/list"})
sock.sendall(encode_frame(77, "raw-corruptor", bad_payload, magic=0x5858))
corrupt_response = read_frame(sock)

good_payload = json.dumps({"jsonrpc": "2.0", "id": "after-corrupt", "method": "initialize", "params": {
    "protocolVersion": "2024-11-05",
    "capabilities": {},
    "clientInfo": {"name": "raw-corruptor", "version": "1.0"},
}})
sock.sendall(encode_frame(78, "raw-corruptor", good_payload))
valid_response = read_frame(sock)
sock.close()

print(json.dumps({
    "initial": initial_response,
    "corrupt": corrupt_response,
    "valid": valid_response,
}))
'''
        result = _exec_cli(svc, vm, _guest_python(script), timeout=60)
        assert result.returncode == 0, result.stderr
        payload = json.loads(result.stdout.strip().splitlines()[-1])
        assert payload["initial"]["stream_id"] == 76
        assert payload["initial"]["payload"]["result"]["serverInfo"]["name"] == (
            "capsem-mcp-mitm-endpoint"
        )
        assert payload["corrupt"]["stream_id"] == 77
        assert payload["corrupt"]["payload"]["error"]["code"] == -32600
        assert payload["valid"]["stream_id"] == 78
        assert payload["valid"]["payload"]["result"]["serverInfo"]["name"] == (
            "capsem-mcp-mitm-endpoint"
        )

        row = _wait_for_mcp_row(
            _session_db(svc, vm),
            lambda r: (
                r["request_id"] == "after-corrupt"
                and r["process_name"] == "raw-corruptor"
            ),
        )
        assert row["method"] == "initialize"
        assert row["policy_action"] == "allow"
    finally:
        if vm is not None:
            _delete_vm(svc, vm)
        svc.stop()


def test_framed_guest_mcp_policy_reload_blocks_existing_connection():
    svc = _start_service()
    vm = None
    proc = None
    try:
        vm = _create_vm(svc, "framed-policy")
        db_path = _session_db(svc, vm)
        script = r'''
import json
import subprocess
import time

proc = subprocess.Popen(
    ["/run/capsem-mcp-server"],
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=subprocess.PIPE,
    text=True,
)

def send(message):
    proc.stdin.write(json.dumps(message) + "\n")
    proc.stdin.flush()
    return json.loads(proc.stdout.readline())

responses = []
responses.append(send({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
    "protocolVersion": "2024-11-05",
    "capabilities": {},
    "clientInfo": {"name": "policy-reload-e2e", "version": "1.0"},
}}))
proc.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
proc.stdin.flush()
responses.append(send({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {
    "name": "local__echo",
    "arguments": {"text": "before-reload"},
}}))
time.sleep(8)
responses.append(send({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {
    "name": "local__echo",
    "arguments": {"text": "after-reload"},
}}))
proc.stdin.close()
proc.wait(timeout=10)
print(json.dumps({"responses": responses, "stderr": proc.stderr.read()}))
'''
        proc = _start_cli_exec(svc, vm, _guest_python(script), timeout=40)

        _wait_for_mcp_row(
            db_path,
            lambda r: r["request_id"] == "2" and r["decision"] == "allowed",
        )

        config_path = svc.tmp_dir / "user.toml"
        config_path.write_text(
            """
[profiles.rules.block_local_echo]
name = "block_local_echo"
action = "block"
priority = 10
match = 'mcp.tool_call.name == "local__echo"'
reason = "test blocks local echo through security rules"
""".lstrip(),
            encoding="utf-8",
        )
        reload_response = svc.client().post("/reload-config", {}, timeout=15)
        assert reload_response["success"] is True

        stdout, stderr = proc.communicate(timeout=60)
        assert proc.returncode == 0, stderr
        responses = _responses_by_id(stdout)
        assert "error" not in responses[2]
        assert responses[3]["error"]["message"].startswith("MCP request blocked by policy")

        denied = _wait_for_mcp_row(
            db_path,
            lambda r: r["request_id"] == "3" and r["decision"] == "denied",
        )
        assert denied["policy_action"] == "deny"
        assert denied["policy_rule"] == "profiles.rules.block_local_echo"
        assert "after-reload" in denied["request_preview"]
    finally:
        if proc is not None and proc.poll() is None:
            proc.kill()
            proc.communicate(timeout=10)
        if vm is not None:
            _delete_vm(svc, vm)
        svc.stop()



def test_framed_guest_mcp_builtin_http_policy_writes_mcp_and_net_rows():
    svc = _start_service()
    vm = None
    try:
        config_path = svc.tmp_dir / "user.toml"
        config_path.write_text(
            """
[profiles.rules.block_builtin_http]
name = "block_builtin_http"
action = "block"
priority = 10
match = 'http.host == "blocked-builtin-http.invalid"'
reason = "test blocks builtin HTTP through security rules"
""".lstrip(),
            encoding="utf-8",
        )
        reload_response = svc.client().post("/reload-config", {}, timeout=15)
        assert reload_response["success"] is True

        vm = _create_vm(svc, "framed-builtin-http")
        script = r'''
import json
import subprocess
import sys

messages = [
    {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2024-11-05",
        "capabilities": {},
        "clientInfo": {"name": "builtin-http-policy-e2e", "version": "1.0"},
    }},
    {"jsonrpc": "2.0", "method": "notifications/initialized"},
    {"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {
        "name": "local__http_headers",
        "arguments": {"url": "https://example.com/", "method": "HEAD"},
    }},
    {"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {
        "name": "local__http_headers",
        "arguments": {"url": "https://blocked-builtin-http.invalid/no-upstream", "method": "HEAD"},
    }},
]

proc = subprocess.run(
    ["/run/capsem-mcp-server"],
    input="\n".join(json.dumps(m) for m in messages) + "\n",
    capture_output=True,
    text=True,
    timeout=45,
)
responses = [json.loads(line) for line in proc.stdout.splitlines() if line.strip()]
print(json.dumps({
    "returncode": proc.returncode,
    "stderr": proc.stderr,
    "responses": responses,
}))
sys.exit(proc.returncode)
'''
        result = _exec_cli(svc, vm, _guest_python(script), timeout=120)
        assert result.returncode == 0, result.stderr
        responses = _responses_by_id(result.stdout)
        assert "Status:" in json.dumps(responses[2]["result"])
        assert "domain blocked by policy: blocked-builtin-http.invalid" in json.dumps(
            responses[3]["result"]
        )

        db_path = _session_db(svc, vm)
        allowed_mcp = _wait_for_mcp_row(
            db_path,
            lambda r: r["request_id"] == "2" and r["tool_name"] == "local__http_headers",
        )
        assert allowed_mcp["decision"] == "allowed"
        blocked_mcp = _wait_for_mcp_row(
            db_path,
            lambda r: r["request_id"] == "3" and r["tool_name"] == "local__http_headers",
        )
        assert blocked_mcp["decision"] == "allowed"
        assert "blocked-builtin-http.invalid" in (blocked_mcp["response_preview"] or "")

        allowed_net = _wait_for_net_row(
            db_path,
            lambda r: r["domain"] == "example.com" and r["method"] == "HEAD",
        )
        assert allowed_net["decision"] == "allowed"
        assert allowed_net["process_name"] == "mcp_builtin"
        assert allowed_net["conn_type"] == "mcp_builtin"
        assert allowed_net["status_code"] is not None

        blocked_net = _wait_for_net_row(
            db_path,
            lambda r: r["domain"] == "blocked-builtin-http.invalid",
        )
        assert blocked_net["decision"] == "denied"
        assert blocked_net["method"] == "HEAD"
        assert blocked_net["path"] == "/no-upstream"
        assert blocked_net["process_name"] == "mcp_builtin"
        assert blocked_net["bytes_sent"] == 0
        assert blocked_net["bytes_received"] == 0
        assert blocked_net["status_code"] is None
    finally:
        if vm is not None:
            _delete_vm(svc, vm)
        svc.stop()


def test_framed_guest_mcp_concurrent_process_attribution():
    svc = _start_service()
    vm = None
    try:
        vm = _create_vm(svc, "framed-procs")
        script = r'''
import concurrent.futures
import json
import os
import subprocess

parents = ["mcp_parent_a", "mcp_parent_b", "mcp_parent_c"]

def run_parent(parent):
    link = f"/tmp/{parent}"
    try:
        os.unlink(link)
    except FileNotFoundError:
        pass
    os.symlink("/bin/sh", link)
    messages = [
        {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": parent, "version": "1.0"},
        }},
        {"jsonrpc": "2.0", "method": "notifications/initialized"},
        {"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {
            "name": "local__echo",
            "arguments": {"text": parent},
        }},
    ]
    proc = subprocess.run(
        [link, "-c", "/run/capsem-mcp-server; true"],
        input="\n".join(json.dumps(m) for m in messages) + "\n",
        capture_output=True,
        text=True,
        timeout=30,
    )
    responses = [json.loads(line) for line in proc.stdout.splitlines() if line.strip()]
    return {"parent": parent, "returncode": proc.returncode, "stderr": proc.stderr, "responses": responses}

with concurrent.futures.ThreadPoolExecutor(max_workers=len(parents)) as pool:
    results = list(pool.map(run_parent, parents))
print(json.dumps({"results": results}))
'''
        result = _exec_cli(svc, vm, _guest_python(script), timeout=90)
        assert result.returncode == 0, result.stderr
        payload = json.loads(result.stdout.strip().splitlines()[-1])
        for item in payload["results"]:
            assert item["returncode"] == 0, item
            by_id = {resp["id"]: resp for resp in item["responses"] if "id" in resp}
            assert item["parent"] in json.dumps(by_id[2]["result"])

        db_path = _session_db(svc, vm)
        for parent in ["mcp_parent_a", "mcp_parent_b", "mcp_parent_c"]:
            row = _wait_for_mcp_row(
                db_path,
                lambda r, parent=parent: (
                    r["method"] == "tools/call"
                    and r["process_name"] == parent
                    and r["request_preview"]
                    and parent in r["request_preview"]
                ),
            )
            assert row["tool_name"] == "local__echo"
            assert row["decision"] == "allowed"
    finally:
        if vm is not None:
            _delete_vm(svc, vm)
        svc.stop()


def test_framed_guest_mcp_external_stdio_tool_and_session_db_row():
    svc = _start_service()
    vm = None
    try:
        fast_server = svc.tmp_dir / "fast_mcp.py"
        fast_server.write_text(
            textwrap.dedent(
                """\
                import json
                import sys

                def respond(req, result=None, error=None):
                    msg = {"jsonrpc": "2.0", "id": req.get("id")}
                    if error is not None:
                        msg["error"] = {"code": -32000, "message": error}
                    else:
                        msg["result"] = result
                    print(json.dumps(msg), flush=True)

                for line in sys.stdin:
                    req = json.loads(line)
                    if "id" not in req:
                        continue
                    method = req.get("method")
                    if method == "initialize":
                        respond(req, {
                            "protocolVersion": "2024-11-05",
                            "capabilities": {"tools": {}},
                            "serverInfo": {"name": "fast-mcp", "version": "1.0"},
                        })
                    elif method == "tools/list":
                        respond(req, {"tools": [{
                            "name": "ping",
                            "description": "Return the input text.",
                            "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}},
                        }]})
                    elif method == "tools/call":
                        text = req.get("params", {}).get("arguments", {}).get("text", "")
                        respond(req, {"content": [{"type": "text", "text": f"fast:{text}"}], "isError": False})
                    else:
                        respond(req, error=f"unknown method: {method}")
                """
            ),
            encoding="utf-8",
        )
        claude_dir = svc.tmp_dir / ".claude"
        claude_dir.mkdir(parents=True, exist_ok=True)
        (claude_dir / "settings.json").write_text(
            json.dumps(
                {
                    "mcpServers": {
                        "fast": {
                            "command": sys.executable,
                            "args": [str(fast_server)],
                        }
                    }
                }
            ),
            encoding="utf-8",
        )

        vm = _create_vm(svc, "framed-external")
        script = r'''
import json
import subprocess
import sys

messages = [
    {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2024-11-05",
        "capabilities": {},
        "clientInfo": {"name": "external-e2e", "version": "1.0"},
    }},
    {"jsonrpc": "2.0", "method": "notifications/initialized"},
    {"jsonrpc": "2.0", "id": 2, "method": "tools/list"},
    {"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {
        "name": "fast__ping",
        "arguments": {"text": "external-ok"},
    }},
]

proc = subprocess.run(
    ["/run/capsem-mcp-server"],
    input="\n".join(json.dumps(m) for m in messages) + "\n",
    capture_output=True,
    text=True,
    timeout=30,
)
responses = [json.loads(line) for line in proc.stdout.splitlines() if line.strip()]
print(json.dumps({"returncode": proc.returncode, "stderr": proc.stderr, "responses": responses}))
sys.exit(proc.returncode)
'''
        result = _exec_cli(svc, vm, _guest_python(script), timeout=90)
        assert result.returncode == 0, result.stderr
        responses = _responses_by_id(result.stdout)
        assert "fast__ping" in json.dumps(responses[2]["result"])
        assert "fast:external-ok" in json.dumps(responses[3]["result"])

        row = _wait_for_mcp_row(
            _session_db(svc, vm),
            lambda r: r["request_id"] == "3" and r["decision"] == "allowed",
        )
        assert row["server_name"] == "fast"
        assert row["tool_name"] == "fast__ping"
        assert "external-ok" in row["request_preview"]
        assert "fast:external-ok" in row["response_preview"]
    finally:
        if vm is not None:
            _delete_vm(svc, vm)
        svc.stop()



def test_framed_guest_mcp_tool_timeout_records_terminal_error(monkeypatch):
    monkeypatch.setenv("CAPSEM_MCP_TOOL_CALL_TIMEOUT_SECS", "1")
    monkeypatch.setenv("CAPSEM_MCP_TOOL_CALL_TIMEOUT_CEILING_SECS", "1")

    svc = _start_service()
    vm = None
    try:
        slow_server = svc.tmp_dir / "slow_mcp.py"
        slow_server.write_text(
            textwrap.dedent(
                """\
                import json
                import sys
                import time

                def respond(req, result=None, error=None):
                    msg = {"jsonrpc": "2.0", "id": req.get("id")}
                    if error is not None:
                        msg["error"] = {"code": -32000, "message": error}
                    else:
                        msg["result"] = result
                    print(json.dumps(msg), flush=True)

                for line in sys.stdin:
                    req = json.loads(line)
                    if "id" not in req:
                        continue
                    method = req.get("method")
                    if method == "initialize":
                        respond(req, {
                            "protocolVersion": "2024-11-05",
                            "capabilities": {"tools": {}},
                            "serverInfo": {"name": "slow-mcp", "version": "1.0"},
                        })
                    elif method == "tools/list":
                        respond(req, {"tools": [{
                            "name": "sleep",
                            "description": "Sleep before responding.",
                            "inputSchema": {"type": "object", "properties": {}},
                        }]})
                    elif method == "tools/call":
                        time.sleep(3)
                        respond(req, {"content": [{"type": "text", "text": "done"}], "isError": False})
                    else:
                        respond(req, error=f"unknown method: {method}")
                """
            ),
            encoding="utf-8",
        )
        claude_dir = svc.tmp_dir / ".claude"
        claude_dir.mkdir(parents=True, exist_ok=True)
        (claude_dir / "settings.json").write_text(
            json.dumps({
                "mcpServers": {
                    "slow": {
                        "command": sys.executable,
                        "args": [str(slow_server)],
                    }
                }
            }),
            encoding="utf-8",
        )

        vm = _create_vm(svc, "framed-timeout")
        script = r'''
import json
import subprocess
import sys

messages = [
    {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2024-11-05",
        "capabilities": {},
        "clientInfo": {"name": "timeout-e2e", "version": "1.0"},
    }},
    {"jsonrpc": "2.0", "method": "notifications/initialized"},
    {"jsonrpc": "2.0", "id": 2, "method": "tools/list"},
    {"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {
        "name": "slow__sleep",
        "arguments": {},
    }},
]

proc = subprocess.run(
    ["/run/capsem-mcp-server"],
    input="\n".join(json.dumps(m) for m in messages) + "\n",
    capture_output=True,
    text=True,
    timeout=20,
)
responses = [json.loads(line) for line in proc.stdout.splitlines() if line.strip()]
print(json.dumps({"returncode": proc.returncode, "stderr": proc.stderr, "responses": responses}))
sys.exit(proc.returncode)
'''
        result = _exec_cli(svc, vm, _guest_python(script), timeout=30)
        assert result.returncode == 0, result.stderr
        responses = _responses_by_id(result.stdout)
        assert "slow__sleep" in json.dumps(responses[2]["result"])
        assert responses[3]["error"]["message"].startswith("MCP request timed out")

        timeout_row = _wait_for_mcp_row(
            _session_db(svc, vm),
            lambda r: r["request_id"] == "3" and r["decision"] == "error",
        )
        assert timeout_row["tool_name"] == "slow__sleep"
        assert timeout_row["policy_action"] == "allow"
        assert "timed out" in timeout_row["error_message"]
    finally:
        if vm is not None:
            _delete_vm(svc, vm)
        svc.stop()


def test_framed_guest_mcp_non_tool_timeout_records_terminal_error(monkeypatch):
    monkeypatch.setenv("CAPSEM_MCP_DEFAULT_TIMEOUT_SECS", "1")

    svc = _start_service()
    vm = None
    try:
        slow_server = svc.tmp_dir / "slow_list_mcp.py"
        slow_server.write_text(
            textwrap.dedent(
                """\
                import json
                import sys
                import time

                def respond(req, result=None, error=None):
                    msg = {"jsonrpc": "2.0", "id": req.get("id")}
                    if error is not None:
                        msg["error"] = {"code": -32000, "message": error}
                    else:
                        msg["result"] = result
                    print(json.dumps(msg), flush=True)

                for line in sys.stdin:
                    req = json.loads(line)
                    if "id" not in req:
                        continue
                    method = req.get("method")
                    if method == "initialize":
                        respond(req, {
                            "protocolVersion": "2024-11-05",
                            "capabilities": {"tools": {}},
                            "serverInfo": {"name": "slow-list-mcp", "version": "1.0"},
                        })
                    elif method == "tools/list":
                        respond(req, {"tools": []})
                    elif method == "resources/list":
                        respond(req, {"resources": [{
                            "uri": "doc://slow",
                            "name": "slow-doc",
                            "description": "Slow resource",
                            "mimeType": "text/plain",
                        }]})
                    elif method == "resources/read":
                        time.sleep(3)
                        respond(req, {"contents": [{
                            "uri": "doc://slow",
                            "mimeType": "text/plain",
                            "text": "too late",
                        }]})
                    elif method == "prompts/list":
                        respond(req, {"prompts": []})
                    elif method == "prompts/get":
                        respond(req, {"tools": []})
                    else:
                        respond(req, error=f"unknown method: {method}")
                """
            ),
            encoding="utf-8",
        )
        claude_dir = svc.tmp_dir / ".claude"
        claude_dir.mkdir(parents=True, exist_ok=True)
        (claude_dir / "settings.json").write_text(
            json.dumps({
                "mcpServers": {
                    "slowlist": {
                        "command": sys.executable,
                        "args": [str(slow_server)],
                    }
                }
            }),
            encoding="utf-8",
        )

        vm = _create_vm(svc, "framed-non-tool-timeout")
        script = r'''
import json
import subprocess
import sys

messages = [
    {"jsonrpc": "2.0", "id": "slow-resource-init", "method": "initialize", "params": {
        "protocolVersion": "2024-11-05",
        "capabilities": {},
        "clientInfo": {"name": "non-tool-timeout-e2e", "version": "1.0"},
    }},
    {"jsonrpc": "2.0", "method": "notifications/initialized"},
    {"jsonrpc": "2.0", "id": "slow-resource-request", "method": "resources/read", "params": {
        "uri": "capsem://slowlist/doc://slow",
    }},
]

proc = subprocess.run(
    ["/run/capsem-mcp-server"],
    input="\n".join(json.dumps(m) for m in messages) + "\n",
    capture_output=True,
    text=True,
    timeout=20,
)
responses = [json.loads(line) for line in proc.stdout.splitlines() if line.strip()]
print(json.dumps({"returncode": proc.returncode, "stderr": proc.stderr, "responses": responses}))
sys.exit(proc.returncode)
'''
        result = _exec_cli(svc, vm, _guest_python(script), timeout=30)
        assert result.returncode == 0, result.stderr
        responses = _responses_by_id(result.stdout)
        assert responses["slow-resource-request"]["error"]["message"].startswith(
            "MCP request timed out"
        )

        timeout_row = _wait_for_mcp_row(
            _session_db(svc, vm),
            lambda r: (
                r["request_id"] == "slow-resource-request" and r["decision"] == "error"
            ),
        )
        assert timeout_row["method"] == "resources/read"
        assert timeout_row["policy_action"] == "allow"
        assert "timed out" in timeout_row["error_message"]
    finally:
        if vm is not None:
            _delete_vm(svc, vm)
        svc.stop()


def test_framed_guest_mcp_reconnects_after_persistent_resume():
    svc = _start_service()
    vm = None
    try:
        vm = _create_vm(svc, "framed-resume", persistent=True)
        first = _exec_cli(
            svc,
            vm,
            _guest_mcp_smoke_command("resume-e2e-before", "before-resume-list"),
            timeout=90,
        )
        assert first.returncode == 0, first.stderr
        assert "local__echo" in json.dumps(_responses_by_id(first.stdout)["before-resume-list"])

        stop_response = svc.client().post(f"/stop/{vm}", {}, timeout=90)
        assert stop_response["success"] is True
        resume_response = svc.client().post(f"/resume/{vm}", {}, timeout=120)
        assert resume_response["id"] == vm
        if not wait_exec_ready(svc.client(), vm):
            pytest.fail(f"VM {vm} never became exec-ready after resume")

        second = _exec_cli(
            svc,
            vm,
            _guest_mcp_smoke_command("resume-e2e-after", "after-resume-list"),
            timeout=90,
        )
        assert second.returncode == 0, second.stderr
        assert "local__echo" in json.dumps(_responses_by_id(second.stdout)["after-resume-list"])

        db_path = _session_db(svc, vm, persistent=True)
        before = _wait_for_mcp_row(
            db_path,
            lambda r: r["request_id"] == "before-resume-list",
        )
        after = _wait_for_mcp_row(
            db_path,
            lambda r: r["request_id"] == "after-resume-list",
        )
        assert before["method"] == "tools/list"
        assert after["method"] == "tools/list"
        assert before["process_name"] == "python3"
        assert after["process_name"] == "python3"
    finally:
        if vm is not None:
            _delete_vm(svc, vm)
        svc.stop()


def test_guest_mcp_legacy_vsock_5003_is_closed():
    svc = _start_service()
    vm = None
    try:
        vm = _create_vm(svc, "mcp-no-5003")
        script = r'''
import json
import socket

result = {"connected": False}
sock = socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM)
sock.settimeout(3)
try:
    sock.connect((socket.VMADDR_CID_HOST, 5003))
    result["connected"] = True
except OSError as exc:
    result["errno"] = exc.errno
    result["error"] = str(exc)
finally:
    sock.close()
print(json.dumps(result))
'''
        result = _exec_cli(svc, vm, _guest_python(script), timeout=30)
        assert result.returncode == 0, result.stderr
        payload = json.loads(result.stdout.strip().splitlines()[-1])
        assert payload["connected"] is False, payload
    finally:
        if vm is not None:
            _delete_vm(svc, vm)
        svc.stop()
