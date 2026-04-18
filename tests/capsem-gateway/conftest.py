"""Shared fixtures for capsem-gateway integration tests."""

import json
import os
import socket
import socketserver
import tempfile
import threading
import uuid
from http.server import BaseHTTPRequestHandler
from pathlib import Path

import pytest

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB
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
        if self.clean_path == "/list" or self.clean_path.startswith("/list?"):
            sandboxes = []
            for vm in MOCK_VMS.values():
                sandboxes.append({
                    "id": vm["id"],
                    "pid": vm["pid"],
                    "status": vm["status"],
                    "persistent": vm["persistent"],
                    "ram_mb": vm["ram_mb"],
                    "cpus": vm["cpus"],
                })
            self._send_json({"sandboxes": sandboxes})
        elif self.clean_path.startswith("/info/"):
            vm_id = self.clean_path.split("/info/", 1)[1].split("?")[0]
            if vm_id in MOCK_VMS:
                self._send_json(MOCK_VMS[vm_id])
            else:
                self._send_error(404, f"sandbox {vm_id} not found")
        elif self.clean_path == "/images":
            self._send_json({"images": []})
        elif self.clean_path.startswith("/logs/"):
            self._send_json({"logs": "mock boot log\n", "serial_logs": None, "process_logs": None})
        else:
            self._send_error(404, f"unknown endpoint: {self.clean_path}")

    def do_POST(self):
        body = self._read_body()
        if self.clean_path == "/provision":
            data = json.loads(body) if body else {}
            vm_id = f"vm-{uuid.uuid4().hex[:8]}"
            self._send_json({"id": vm_id})
        elif self.clean_path.startswith("/exec/"):
            data = json.loads(body) if body else {}
            cmd = data.get("command", "")
            self._send_json({"stdout": f"mock: {cmd}\n", "stderr": "", "exit_code": 0})
        elif self.clean_path.startswith("/stop/"):
            self._send_json({"ok": True})
        elif self.clean_path.startswith("/write_file/"):
            self._send_json({"success": True})
        elif self.clean_path.startswith("/read_file/"):
            self._send_json({"content": "mock file content"})
        elif self.clean_path.startswith("/inspect/"):
            self._send_json({"columns": [], "rows": []})
        elif self.clean_path.startswith("/persist/"):
            self._send_json({"ok": True})
        elif self.clean_path == "/purge":
            self._send_json({"purged": 0, "persistent_purged": 0, "ephemeral_purged": 0})
        elif self.clean_path == "/run":
            self._send_json({"stdout": "mock run output\n", "stderr": "", "exit_code": 0})
        elif self.clean_path.startswith("/resume/"):
            self._send_json({"id": "vm-resumed"})
        elif self.clean_path.startswith("/fork/"):
            data = json.loads(body) if body else {}
            self._send_json({"name": data.get("name", "fork"), "size_bytes": 1024})
        elif self.clean_path == "/reload-config":
            self._send_json({"ok": True})
        elif self.clean_path == "/echo":
            # Echo back the request body for proxy testing
            self.send_response(200)
            self.send_header("Content-Type", "application/octet-stream")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        else:
            self._send_error(404, f"unknown endpoint: {self.clean_path}")

    def do_DELETE(self):
        if self.clean_path.startswith("/delete/"):
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
