"""Shared fixtures for capsem-gateway integration tests.

Scope: gateway layer only. These tests cover the TCP-to-UDS proxy shell --
routing, auth, CORS, lifecycle, terminal WebSocket handshake, SPA static
serving -- using a pytest-local `MockServiceHandler` as the UDS backend.
They deliberately do NOT verify that the downstream capsem-service
endpoints behave correctly under real inputs; that correctness is owned
by:

  tests/capsem-service/    (every HTTP handler against the real service)
  tests/capsem-mcp/        (every #[tool] in capsem-mcp against a live
                            capsem-mcp -> capsem-service -> VM chain)
  tests/capsem-e2e/        (full CLI -> gateway -> service -> VM paths
                            for a handful of flagship flows)

If a gateway-proxied response shape changes (e.g. /vms/list returns a new
field), update the mock here AND the corresponding service test in
tests/capsem-service/. If you find yourself writing an assertion about
what the service should return, you're in the wrong directory.
"""

import json
import os
import socketserver
import tempfile
import threading
import uuid
from http.server import BaseHTTPRequestHandler
from pathlib import Path

import pytest

from helpers.constants import CODE_PROFILE_ID, DEFAULT_CPUS, DEFAULT_RAM_MB
from helpers.gateway import GatewayInstance, TcpHttpClient

pytestmark = pytest.mark.gateway

# --- Mock capsem-service on UDS ---

MOCK_VMS = {
    "vm-001": {
        "id": "vm-001",
        "pid": 100,
        "name": "dev",
        "status": "Running",
        "persistent": True,
        "ram_mb": DEFAULT_RAM_MB,
        "cpus": DEFAULT_CPUS,
        "version": "0.16.1",
    },
    "vm-002": {
        "id": "vm-002",
        "pid": 200,
        "name": None,
        "status": "Running",
        "persistent": False,
        "ram_mb": DEFAULT_RAM_MB * 2,
        "cpus": DEFAULT_CPUS * 2,
        "version": "0.16.1",
    },
}


class MockServiceHandler(BaseHTTPRequestHandler):
    """HTTP handler mimicking capsem-service responses."""

    def log_message(self, format, *args):
        pass  # Suppress default logging

    @property
    def clean_path(self):
        """Strip http://localhost prefix that hyper sends over UDS."""
        p = self.path
        if p.startswith("http://localhost"):
            p = p[len("http://localhost"):]
        return p

    def _read_body(self):
        length = int(self.headers.get("Content-Length", 0))
        return self.rfile.read(length) if length > 0 else b""

    def _send_json(self, data, status=200):
        body = json.dumps(data).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _send_error(self, status, msg):
        self._send_json({"error": msg}, status=status)

    def do_GET(self):
        path_only = self.clean_path.split("?", 1)[0]
        if path_only == "/vms/list":
            sandboxes = []
            for vm in MOCK_VMS.values():
                sandboxes.append({
                    "id": vm["id"],
                    "pid": vm["pid"],
                    "profile_id": CODE_PROFILE_ID,
                    "status": vm["status"],
                    "persistent": vm["persistent"],
                    "ram_mb": vm["ram_mb"],
                    "cpus": vm["cpus"],
                    "available_actions": ["pause", "stop", "fork", "delete"],
                })
            self._send_json({"sandboxes": sandboxes})
        elif path_only.startswith("/vms/") and path_only.endswith("/info"):
            vm_id = path_only.split("/vms/", 1)[1].rsplit("/info", 1)[0]
            if vm_id in MOCK_VMS:
                self._send_json(MOCK_VMS[vm_id])
            else:
                self._send_error(404, f"sandbox {vm_id} not found")
        elif path_only.startswith("/vms/") and path_only.endswith("/snapshots/status"):
            vm_id = path_only.split("/vms/", 1)[1].rsplit("/snapshots/status", 1)[0]
            if vm_id in MOCK_VMS:
                self._send_json({
                    "total": 1,
                    "auto_count": 1,
                    "manual_count": 0,
                    "manual_available": 12,
                    "snapshots": [
                        {
                            "checkpoint": "checkpoint-0",
                            "slot": 0,
                            "origin": "auto",
                            "timestamp": "unix:1700000000",
                        }
                    ],
                })
            else:
                self._send_error(404, f"sandbox {vm_id} not found")
        elif path_only.startswith("/vms/") and path_only.endswith("/snapshots/list"):
            vm_id = path_only.split("/vms/", 1)[1].rsplit("/snapshots/list", 1)[0]
            if vm_id in MOCK_VMS:
                self._send_json({
                    "total": 1,
                    "snapshots": [
                        {
                            "checkpoint": "checkpoint-0",
                            "slot": 0,
                            "origin": "auto",
                            "timestamp": "unix:1700000000",
                        }
                    ],
                })
            else:
                self._send_error(404, f"sandbox {vm_id} not found")
        elif path_only.startswith("/vms/") and path_only.endswith("/status"):
            vm_id = path_only.split("/vms/", 1)[1].rsplit("/status", 1)[0]
            if vm_id in MOCK_VMS:
                vm = MOCK_VMS[vm_id]
                self._send_json({
                    "id": vm["id"],
                    "profile_id": CODE_PROFILE_ID,
                    "status": vm["status"],
                    "pid": vm["pid"],
                    "persistent": vm["persistent"],
                    "available_actions": ["pause", "stop", "fork", "delete"],
                })
            else:
                self._send_error(404, f"sandbox {vm_id} not found")
        elif path_only.startswith("/vms/") and path_only.endswith("/logs"):
            self._send_json({"logs": "mock boot log\n", "serial_logs": None, "process_logs": None})
        elif path_only.startswith("/vms/") and path_only.endswith("/files/list"):
            self._send_json({"entries": []})
        elif path_only.startswith("/vms/") and path_only.endswith("/files/content"):
            body = b"mock file bytes"
            self.send_response(200)
            self.send_header("Content-Type", "text/plain")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        elif path_only == "/profiles/status":
            self._send_json({
                "source": "directory",
                "profile_count": 2,
                "ready_count": 1,
                "asset_manifest": {
                    "origin": "package",
                    "path": "/Users/test/.capsem/assets/manifest.json",
                    "origin_path": "/Users/test/.capsem/assets/manifest-origin.json",
                    "origin_source": "file:///tmp/corp/manifest.json",
                    "packaged_at": "2026-06-13T00:00:00Z",
                    "blake3": "0123456789abcdef",
                    "validation_status": "valid",
                    "refresh_policy": "24h",
                    "assets_current": "2026.0613.1",
                    "binaries_current": "1.3.0",
                },
                "profiles": [
                    {
                        "id": CODE_PROFILE_ID,
                        "name": "Code",
                        "description": "Optimized for coding and long-running agents.",
                        "ready": True,
                        "current_arch": "arm64",
                        "missing_assets": [],
                        "invalid_assets": [],
                        "invalid_files": [],
                        "errors": [],
                        "asset_count": 3,
                    },
                    {
                        "id": "co-work",
                        "name": "Co-work",
                        "description": "Shared profile for collaborative agent sessions.",
                        "ready": False,
                        "current_arch": "arm64",
                        "missing_assets": [{"kind": "rootfs", "path": "/missing/rootfs.erofs", "valid": False}],
                        "invalid_assets": [],
                        "invalid_files": [],
                        "errors": ["missing rootfs"],
                        "asset_count": 3,
                    },
                ],
            })
        else:
            self._send_error(404, f"unknown endpoint: {self.clean_path}")

    def do_POST(self):
        body = self._read_body()
        path_only = self.clean_path.split("?", 1)[0]
        if path_only == "/vms/create":
            data = json.loads(body) if body else {}
            if data.get("profile_id") != CODE_PROFILE_ID:
                self._send_error(400, "profile_id is required")
                return
            vm_id = f"vm-{uuid.uuid4().hex[:8]}"
            self._send_json({"id": vm_id})
        elif path_only.startswith("/vms/") and path_only.endswith("/exec"):
            data = json.loads(body) if body else {}
            cmd = data.get("command", "")
            self._send_json({"stdout": f"mock: {cmd}\n", "stderr": "", "exit_code": 0})
        elif path_only.startswith("/vms/") and path_only.endswith("/stop"):
            self._send_json({"ok": True})
        elif path_only.startswith("/vms/") and path_only.endswith("/files/write"):
            self._send_json({"success": True})
        elif path_only.startswith("/vms/") and path_only.endswith("/files/read"):
            self._send_json({"content": "mock file content"})
        elif path_only.startswith("/vms/") and path_only.endswith("/files/content"):
            self._send_json({"success": True, "size": len(body)})
        elif path_only.startswith("/vms/") and path_only.endswith("/save"):
            self._send_json({"ok": True})
        elif path_only == "/purge":
            self._send_json({"purged": 0, "persistent_purged": 0, "ephemeral_purged": 0})
        elif path_only == "/run":
            data = json.loads(body) if body else {}
            if data.get("profile_id") != CODE_PROFILE_ID:
                self._send_error(400, "profile_id is required")
                return
            self._send_json({"stdout": "mock run output\n", "stderr": "", "exit_code": 0})
        elif path_only.startswith("/vms/") and path_only.endswith("/resume"):
            self._send_json({"id": "vm-resumed"})
        elif path_only.startswith("/vms/") and path_only.endswith("/fork"):
            data = json.loads(body) if body else {}
            self._send_json({"name": data.get("name", "fork"), "size_bytes": 1024})
        elif path_only.startswith("/profiles/") and path_only.endswith("/reload"):
            self._send_json({"ok": True})
        elif path_only == "/echo":
            # Echo back the request body for proxy testing
            self.send_response(200)
            self.send_header("Content-Type", "application/octet-stream")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        else:
            self._send_error(404, f"unknown endpoint: {self.clean_path}")

    def do_DELETE(self):
        if self.clean_path.startswith("/vms/") and self.clean_path.endswith("/delete"):
            self._send_json({"ok": True})
        elif self.clean_path.startswith("/images/"):
            self._send_json({"ok": True})
        else:
            self._send_error(404, f"unknown endpoint: {self.clean_path}")


class UnixStreamServer(socketserver.UnixStreamServer):
    allow_reuse_address = True


class MockServiceServer:
    """HTTP server on Unix socket mimicking capsem-service."""

    def __init__(self):
        self.tmp_dir = tempfile.mkdtemp(prefix="capsem-mock-svc-")
        self.socket_path = os.path.join(self.tmp_dir, "service.sock")
        self._server = None
        self._thread = None

    def start(self):
        self._server = UnixStreamServer(self.socket_path, MockServiceHandler)
        self._thread = threading.Thread(target=self._server.serve_forever, daemon=True)
        self._thread.start()

    def stop(self):
        if self._server:
            self._server.shutdown()
            self._server.server_close()
        if os.path.exists(self.socket_path):
            os.unlink(self.socket_path)


# --- Fixtures ---


@pytest.fixture(scope="session")
def mock_service():
    """Start a mock capsem-service on a Unix socket."""
    svc = MockServiceServer()
    svc.start()
    yield svc
    svc.stop()


@pytest.fixture(scope="session")
def gateway_env(mock_service):
    """Start capsem-gateway binary pointing at the mock UDS."""
    gw = GatewayInstance(uds_path=mock_service.socket_path)
    gw.start()
    yield gw
    gw.stop()


@pytest.fixture
def gw_client(gateway_env):
    """TcpHttpClient with valid auth token."""
    return TcpHttpClient(gateway_env.base_url, gateway_env.token)


@pytest.fixture(scope="session")
def frontend_dir():
    """Create a temp dir with mock frontend build artifacts."""
    d = Path(tempfile.mkdtemp(prefix="capsem-frontend-test-"))
    (d / "index.html").write_text(
        '<!DOCTYPE html><html><head>'
        '<link rel="stylesheet" href="/app/_astro/style.abc.css">'
        '</head><body>'
        '<script type="module" src="/app/_astro/app.xyz.js"></script>'
        '</body></html>'
    )
    astro = d / "_astro"
    astro.mkdir()
    (astro / "style.abc.css").write_text("body { color: red; }")
    (astro / "app.xyz.js").write_text("console.log('capsem');")
    (d / "favicon.ico").write_bytes(b"\x00\x00\x01\x00")
    fonts = d / "fonts"
    fonts.mkdir()
    (fonts / "inter.woff2").write_bytes(b"\x00woff2")
    vm = d / "vm" / "terminal"
    vm.mkdir(parents=True)
    (vm / "index.html").write_text("<html><body>terminal</body></html>")
    yield d
    import shutil
    shutil.rmtree(d, ignore_errors=True)


@pytest.fixture(scope="session")
def frontend_gateway_env(mock_service, frontend_dir):
    """Gateway started with --frontend-dir pointing at mock assets."""
    gw = GatewayInstance(
        uds_path=mock_service.socket_path,
        frontend_dir=frontend_dir,
    )
    gw.start()
    yield gw
    gw.stop()


@pytest.fixture
def fe_client(frontend_gateway_env):
    """TcpHttpClient for the frontend-enabled gateway."""
    return TcpHttpClient(frontend_gateway_env.base_url,
                         frontend_gateway_env.token)
