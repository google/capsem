"""Shared fixtures for capsem-mcp integration tests.

Provides: capsem_service (session), mcp_session (function),
shared_vm (session), fresh_vm (function factory).

Uses CAPSEM_UDS_PATH env var so the test service runs on its own socket
without touching the dev service or requiring HOME hacking.
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import uuid

import pytest

from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent.parent))

from helpers.constants import EXEC_READY_TIMEOUT
from helpers.mcp import content_text, parse_content, wait_exec_ready as mcp_wait_exec_ready

PROJECT_ROOT = Path(__file__).parent.parent.parent
MCP_BINARY = PROJECT_ROOT / "target/debug/capsem-mcp"
SERVICE_BINARY = PROJECT_ROOT / "target/debug/capsem-service"
PROCESS_BINARY = PROJECT_ROOT / "target/debug/capsem-process"
ASSETS_DIR = PROJECT_ROOT / "assets"


class McpSession:
    """Live JSON-RPC connection to the MCP server over stdio."""

    def __init__(self, proc):
        self.proc = proc
        self.req_id = 1

    def request(self, method, params=None):
        req = {
            "jsonrpc": "2.0",
            "method": method,
            "params": params or {},
            "id": self.req_id,
        }
        self.proc.stdin.write(json.dumps(req) + "\n")
        self.proc.stdin.flush()
        self.req_id += 1

        resp_line = self.proc.stdout.readline()
        if not resp_line:
            raise EOFError("MCP server closed stdout unexpectedly")
        return json.loads(resp_line)

    def notify(self, method, params=None):
        req = {
            "jsonrpc": "2.0",
            "method": method,
            "params": params or {},
        }
        self.proc.stdin.write(json.dumps(req) + "\n")
        self.proc.stdin.flush()

    def call_tool(self, name, args=None):
        """Call a tool, assert success, return result."""
        resp = self.request("tools/call", {"name": name, "arguments": args or {}})
        assert "error" not in resp, f"JSON-RPC error: {resp.get('error')}"
        assert not resp["result"].get("isError"), (
            f"Tool error: {resp['result'].get('content')}"
        )
        return resp["result"]

    def call_tool_raw(self, name, args=None):
        """Call a tool, return raw response (no assertions)."""
        return self.request("tools/call", {"name": name, "arguments": args or {}})


def _make_mcp_session(uds_path):
    """Spawn capsem-mcp pointed at a specific socket, perform handshake."""
    env = os.environ.copy()
    env["CAPSEM_UDS_PATH"] = str(uds_path)
    env["CAPSEM_RUN_DIR"] = str(Path(uds_path).parent)

    proc = subprocess.Popen(
        [str(MCP_BINARY)],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=sys.stderr,
        text=True,
        bufsize=1,
        env=env,
    )

    session = McpSession(proc)
    session.request("initialize", {
        "protocolVersion": "2024-11-05",
        "capabilities": {},
        "clientInfo": {"name": "pytest-mcp", "version": "1.0"},
    })
    session.notify("notifications/initialized")
    return session, proc


def _kill_proc(proc, timeout=5):
    proc.terminate()
    try:
        proc.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait()


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@pytest.fixture(scope="session")
def capsem_service():
    """Start a dedicated capsem-service on a random socket.

    Fully isolated -- does not touch the dev service or user HOME.
    Temp directory is cleaned up on teardown.
    """
    from helpers.sign import sign_binary
    sign_binary(PROCESS_BINARY)
    sign_binary(SERVICE_BINARY)

    tmp_dir = Path(tempfile.mkdtemp(prefix="capsem-test-"))
    uds_path = tmp_dir / f"service-{uuid.uuid4().hex[:8]}.sock"

    arch = "arm64" if os.uname().machine == "arm64" else "x86_64"
    assets_dir = ASSETS_DIR / arch

    env = os.environ.copy()
    env["RUST_LOG"] = "debug"
    env["CAPSEM_RUN_DIR"] = str(tmp_dir)

    log_path = tmp_dir / "service.log"
    stderr_path = tmp_dir / "service.stderr.log"
    stderr_file = open(stderr_path, "w")

    proc = subprocess.Popen(
        [
            str(SERVICE_BINARY),
            "--uds-path", str(uds_path),
            "--assets-dir", str(assets_dir),
            "--process-binary", str(PROCESS_BINARY),
            "--foreground",
        ],
        env=env,
        stdout=subprocess.DEVNULL,
        stderr=stderr_file,
    )

    start = time.time()
    while time.time() - start < 15:
        if uds_path.exists():
            break
        time.sleep(0.5)
    else:
        proc.terminate()
        stderr_file.close()
        if log_path.exists():
            print(f"\n--- SERVICE LOG ---\n{log_path.read_text()}\n---",
                  file=sys.stderr)
        if stderr_path.exists():
            print(f"\n--- SERVICE STDERR ---\n{stderr_path.read_text()}\n---",
                  file=sys.stderr)
        raise RuntimeError("capsem-service failed to create socket within 15s")

    print(f"SERVICE LOG DIR: {log_path}", file=sys.stderr)

    yield uds_path

    proc.terminate()
    try:
        proc.wait(timeout=10)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait()
    
    stderr_file.close()
    
    if log_path.exists():
        print(f"\n--- SERVICE LOG ---\n{log_path.read_text()}\n---", file=sys.stderr)
    if stderr_path.exists():
        print(f"\n--- SERVICE STDERR ---\n{stderr_path.read_text()}\n---", file=sys.stderr)

    # shutil.rmtree(tmp_dir, ignore_errors=True)


@pytest.fixture
def mcp_session(capsem_service):
    """Fresh MCP session with completed handshake (per-test)."""
    session, proc = _make_mcp_session(capsem_service)
    yield session
    _kill_proc(proc)


@pytest.fixture(scope="session")
def shared_vm(capsem_service):
    """One long-lived VM for non-destructive tests (exec, read, info, inspect).

    Session-scoped: boots once, shared across all test files in this directory.
    """
    session, proc = _make_mcp_session(capsem_service)

    vm_name = f"shared-{uuid.uuid4().hex[:8]}"
    session.call_tool("capsem_create", {"name": vm_name})

    if not mcp_wait_exec_ready(session, vm_name, timeout=EXEC_READY_TIMEOUT):
        _kill_proc(proc)
        pytest.fail(f"Shared VM {vm_name} never became exec-ready")

    yield vm_name, session

    try:
        session.call_tool("capsem_delete", {"id": vm_name})
    except Exception:
        pass
    _kill_proc(proc)


@pytest.fixture
def fresh_vm(mcp_session):
    """Factory: creates a uniquely named VM, deletes it on teardown."""
    created = []

    def _create(name=None, **kwargs):
        vm_name = name or f"test-{uuid.uuid4().hex[:8]}"
        args = {"name": vm_name, **kwargs}
        mcp_session.call_tool("capsem_create", args)
        created.append(vm_name)
        return vm_name

    yield _create

    for vm_id in created:
        try:
            mcp_session.call_tool("capsem_delete", {"id": vm_id})
        except Exception:
            pass
