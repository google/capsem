"""Real guest -> Core -> confined native MCP -> JSON/SSE payload boundaries."""

from __future__ import annotations

import base64
import json
import queue
import shlex
import subprocess
import threading
import tomllib
from contextlib import closing, contextmanager, suppress
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import pytest
import tomli_w
from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB, host_bin_root
from helpers.service import (
    ServiceInstance,
    vm_name,
    vm_session_db_path,
    wait_exec_ready,
)
from helpers.session_ledger import open_session_ledger

pytestmark = pytest.mark.e2e
ROOT = Path(__file__).resolve().parents[2]
PARENT_NAME = "mcp_payload_" + "x" * (128 - len("mcp_payload_"))


def encode(message):
    return json.dumps(message, separators=(",", ":"), ensure_ascii=False).encode()


@contextmanager
def upstream(payload_limit):
    calls = []
    lock = threading.Lock()
    replies = queue.Queue()
    empty = {"jsonrpc": "2.0", "result": {"content": [{"type": "text", "text": ""}]}}
    text_bytes = payload_limit - len(encode(empty))

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, format, *args):
            pass

        def do_GET(self):
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.end_headers()
            while (reply := replies.get()) is not None:
                body, oversized = reply
                try:
                    self.wfile.write(b"data: " + body + b"\r\n\r\n")
                    self.wfile.flush()
                except (BrokenPipeError, ConnectionResetError):
                    return
                if oversized:
                    # The production receiver drops a refused stream immediately.
                    # Retire this fixture writer too, before its replacement GET
                    # consumes the next reply.
                    return

        def do_POST(self):
            message = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            if "id" not in message:
                self.send_response(202)
                self.end_headers()
                return
            method = message["method"]
            mode = "json"
            if method == "initialize":
                result = {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {"tools": {}, "resources": {}, "prompts": {}},
                    "serverInfo": {"name": "payload-fixture", "version": "1"},
                }
            elif method == "tools/list":
                result = {"tools": [{"name": "boundary", "inputSchema": {"type": "object"}}]}
            elif method == "resources/list":
                result = {"resources": []}
            elif method == "prompts/list":
                result = {"prompts": []}
            elif method == "tools/call":
                assert message["params"]["name"] == "boundary"
                args = message["params"]["arguments"]
                mode = args["mode"]
                with lock:
                    calls.append((mode, bool(args.get("oversized"))))
                text = (
                    "after refusal"
                    if mode == "echo" or args.get("echo")
                    else "r" * (text_bytes + int(bool(args.get("oversized"))))
                )
                result = {"content": [{"type": "text", "text": text}]}
            else:
                raise AssertionError(f"unexpected method {method}")
            body = encode({"jsonrpc": "2.0", "id": message["id"], "result": result})
            if mode == "get":
                replies.put((body, bool(message["params"]["arguments"].get("oversized"))))
                self.send_response(202)
                self.end_headers()
                return
            if mode == "sse":
                body = b"data: " + body + b"\r\n\r\n"
            self.send_response(200)
            self.send_header("Mcp-Session-Id", "payload-fixture-session")
            self.send_header(
                "Content-Type",
                "text/event-stream" if mode == "sse" else "application/json",
            )
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            with suppress(BrokenPipeError, ConnectionResetError):
                self.wfile.write(body)

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}/mcp", text_bytes, calls
    finally:
        replies.put(None)
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


def test_real_guest_mcp_preserves_payload_bounds_and_recovers_after_refusals():
    limits = tomllib.loads((ROOT / "config/data/runtime-limits.toml").read_text())["mcp"]
    with upstream(limits["payload_max_bytes"]) as (url, text_bytes, calls):
        service = ServiceInstance(isolated_corp_config=True)
        vm_id = None
        try:
            (service.home_dir / "settings.toml").write_text(
                tomli_w.dumps(
                    {
                        "mcp": {
                            "servers": [
                                {"name": "payload", "url": url, "enabled": True},
                            ]
                        }
                    }
                )
            )
            service.start()
            client = service.client()
            created = client.post(
                "/vms/create",
                {
                    "name": vm_name("mcp-boundary"),
                    "ram_mb": DEFAULT_RAM_MB,
                    "cpus": DEFAULT_CPUS,
                },
                timeout=120,
            )
            vm_id = created["id"]
            assert wait_exec_ready(client, vm_id)
            script = (ROOT / "tests/fixtures/mcp/payload_bounds_probe.py").read_bytes()
            encoded = base64.b64encode(script).decode()
            bootstrap = (
                "import base64,os,sys;"
                f"path='/tmp/{PARENT_NAME}';os.symlink(sys.executable,path);"
                f"os.execv(path,[path,'-c',base64.b64decode({encoded!r}).decode(),*sys.argv[1:]])"
            )
            command = f"python3 -c {shlex.quote(bootstrap)} {limits['payload_max_bytes']} {limits['request_id_max_bytes']} {text_bytes}"
            response = subprocess.run(
                [
                    str(host_bin_root() / "capsem"),
                    "--uds-path",
                    str(service.uds_path),
                    "exec",
                    "--timeout",
                    "120",
                    vm_id,
                    command,
                ],
                capture_output=True,
                text=True,
                timeout=150,
                check=False,
            )
            assert response.returncode == 0, (response.stdout, response.stderr)
            summary = json.loads(response.stdout.strip())
            assert summary == {
                "boundary_bytes": text_bytes,
                "complete_modes": ["json", "sse", "get"],
                "local_refusals": 2,
            }
            assert sorted(calls) == sorted(
                [
                    ("json", False),
                    ("json", True),
                    ("sse", False),
                    ("sse", True),
                    ("get", False),
                    ("get", True),
                    ("get", False),
                    ("echo", False),
                    ("echo", False),
                ]
            )
            db_path = vm_session_db_path(service.tmp_dir, client, vm_id)
            client.post(f"/vms/{vm_id}/stop", {}, timeout=60)
            with closing(open_session_ledger(db_path)) as db:
                rows = db.execute(
                    "SELECT process_name, transport FROM tool_calls WHERE origin='mcp' AND server_name='payload'"
                ).fetchall()
            assert len(rows) == 9, rows
            assert all(tuple(row) == (PARENT_NAME, "vsock_frame") for row in rows), rows
        finally:
            if vm_id is not None:
                with suppress(Exception):
                    service.client().delete(f"/vms/{vm_id}/delete", timeout=60)
            service.stop()
