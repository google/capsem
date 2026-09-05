"""Shared gateway startup helper for integration tests.

Starts capsem-gateway pointing at a given UDS path (either a mock or real service).
Reads the generated token from the runtime file for authenticated requests.
"""

import json
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from log_streams import read_log_stream

from build_system.scripts.release.release_test_binary import ensure_host_test_binary

from .constants import BIN_DIR
from .http_transport import Transport

PROJECT_ROOT = Path(__file__).parent.parent.parent
GATEWAY_BINARY = BIN_DIR / "capsem-gateway"
GATEWAY_SOURCE_PATHS = [
    PROJECT_ROOT / "crates" / "capsem-gateway" / "src" / "main.rs",
    PROJECT_ROOT / "crates" / "capsem-gateway" / "src" / "proxy.rs",
    PROJECT_ROOT / "crates" / "capsem-gateway" / "src" / "status.rs",
]


def _ensure_gateway_binary_current() -> None:
    ensure_host_test_binary(
        GATEWAY_BINARY,
        source_paths=GATEWAY_SOURCE_PATHS,
        build_command=("cargo", "build", "-p", "capsem-gateway"),
        project_root=PROJECT_ROOT,
    )


class GatewayInstance:
    """A running capsem-gateway on an isolated temp dir."""

    def __init__(self, uds_path: str | Path, port: int = 0, frontend_dir: str | Path | None = None):
        self.tmp_dir = Path(tempfile.mkdtemp(prefix="capsem-gw-test-"))
        self.uds_path = str(uds_path)
        self._port = port
        self.frontend_dir = str(frontend_dir) if frontend_dir else None
        self.proc = None
        self._log_file = None
        self._stdio_log_path = self.tmp_dir / "gateway-stdio.log"
        self._log_path = self.tmp_dir / ".capsem" / "run" / "gateway.log"
        self.token = ""
        self.port = port

    def start(self):
        _ensure_gateway_binary_current()
        if not GATEWAY_BINARY.exists():
            raise FileNotFoundError(
                f"Gateway binary not found: {GATEWAY_BINARY}. Run 'cargo build -p capsem-gateway'."
            )

        # Pick a free port if not specified
        if self._port == 0:
            import socket

            with socket.socket() as s:
                s.bind(("127.0.0.1", 0))
                self._port = s.getsockname()[1]

        env = os.environ.copy()
        env["RUST_LOG"] = "capsem_gateway=debug"
        # Override HOME so runtime files go to our temp dir
        env["HOME"] = str(self.tmp_dir)

        # Create the run dir where gateway will write its files
        run_dir = self.tmp_dir / ".capsem" / "run"
        run_dir.mkdir(parents=True, exist_ok=True)

        log_path = self._log_path
        print(f"GATEWAY LOG: {log_path}")
        self._log_file = open(self._stdio_log_path, "w")  # noqa: SIM115 -- handed to Popen; must outlive this statement

        # capsem-gateway refuses to run without a live parent service (see
        # capsem-guard). Standalone test invocations pass the pytest worker PID
        # as the parent so the guard's parent-watch is satisfied; when pytest
        # exits, the gateway exits with it. --run-dir is passed explicitly so
        # the token/port/pid/lock files land in the test tmp dir, isolating
        # parallel workers from each other's singleton lock.
        cmd = [
            str(GATEWAY_BINARY),
            "--port",
            str(self._port),
            "--uds-path",
            self.uds_path,
            "--run-dir",
            str(run_dir),
            "--parent-pid",
            str(os.getpid()),
        ]
        if self.frontend_dir:
            cmd += ["--frontend-dir", self.frontend_dir]

        self.proc = subprocess.Popen(
            cmd,
            env=env,
            stdout=self._log_file,
            stderr=self._log_file,
        )

        # Wait for gateway to start and write runtime files
        token_path = run_dir / "gateway.token"
        port_path = run_dir / "gateway.port"
        start = time.time()
        while time.time() - start < 10:
            if token_path.exists() and port_path.exists():
                self.token = token_path.read_text().strip()
                self.port = int(port_path.read_text().strip())
                # Verify HTTP health check responds
                probe = Transport(host="127.0.0.1", port=self.port)
                try:
                    _, _, body = probe.request("GET", "/health", timeout=2)
                    if b"ok" in body.lower():
                        return
                except Exception:
                    pass
                finally:
                    probe.close()
            time.sleep(0.2)

        self.stop()
        gateway_log = read_log_stream(log_path)
        if gateway_log:
            print(f"\n--- GATEWAY LOG ---\n{gateway_log}\n---", file=sys.stderr)
        if self._stdio_log_path.exists():
            print(
                f"\n--- GATEWAY STDIO ---\n{self._stdio_log_path.read_text()}\n---",
                file=sys.stderr,
            )
        raise RuntimeError("capsem-gateway failed to start within 10s")

    def stop(self):
        if self.proc:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait()
            self.proc = None
        if self._log_file:
            self._log_file.close()
            self._log_file = None

    def stop_and_read_log(self) -> str:
        """Stop the gateway so Rust's stdout/stderr log buffer is flushed.

        `gateway.log` names a daily-rotated stream, so the bare name is empty
        once it has rotated. Reading it directly returned "" and callers then
        asserted against an empty string, reporting a gateway that logged
        nothing when it had logged normally into `gateway.<date>.log`.
        """
        self.stop()
        return read_log_stream(self._log_path)

    @property
    def base_url(self) -> str:
        return f"http://127.0.0.1:{self.port}"

    @property
    def auth_header(self) -> str:
        return f"Bearer {self.token}"

    @property
    def run_dir(self) -> Path:
        return self.tmp_dir / ".capsem" / "run"

    @property
    def log_path(self) -> Path:
        return self._log_path


class TcpHttpClient:
    """HTTP client for talking to the gateway over TCP with auth."""

    def __init__(self, base_url: str, token: str | None):
        self.base_url = base_url
        self.token = token
        host, _, port = base_url.removeprefix("http://").partition(":")
        self._transport = Transport(host=host, port=int(port))

    def _headers(self, use_auth, extra=None):
        headers = {"Content-Type": "application/json", **(extra or {})}
        if use_auth:
            headers["Authorization"] = f"Bearer {self.token}"
        return headers

    def call(self, method, path, *, body=None, headers=None, use_auth=True, timeout=30):
        """One request: (status, lower-cased response headers, body bytes).

        A 4xx or 5xx is a response, not an error. `body` is sent as given
        (bytes); `headers` are added to (and may override) the defaults.
        """
        return self._transport.request(
            method, path, headers=self._headers(use_auth, headers), body=body, timeout=timeout
        )

    def call_json(self, method, path, body: object = None, *, use_auth=True, timeout=30):
        """(status, payload): JSON when the body parses, the text when it does
        not, None when it is empty."""
        payload = None if body is None else json.dumps(body).encode()
        status, _, data = self.call(method, path, body=payload, use_auth=use_auth, timeout=timeout)
        text = data.decode(errors="replace")
        if not text.strip():
            return status, None
        try:
            return status, json.loads(text)
        except json.JSONDecodeError:
            return status, text

    def _raw(self, method, path, body=None, timeout=30, use_auth=True, extra_headers=None):
        payload = None if body is None else json.dumps(body).encode()
        status, _, data = self.call(
            method, path, body=payload, headers=extra_headers, use_auth=use_auth, timeout=timeout
        )
        return status, data

    def _request(self, method, path, body=None, timeout=30, use_auth=True):
        _, data = self._raw(method, path, body, timeout=timeout, use_auth=use_auth)
        if not data.strip():
            return None
        return json.loads(data)

    def get(self, path, timeout=30, use_auth=True):
        return self._request("GET", path, timeout=timeout, use_auth=use_auth)

    def post(self, path, body=None, timeout=60, use_auth=True):
        return self._request("POST", path, body, timeout=timeout, use_auth=use_auth)

    def patch(self, path, body=None, timeout=60, use_auth=True):
        return self._request("PATCH", path, body, timeout=timeout, use_auth=use_auth)

    def delete(self, path, timeout=30, use_auth=True):
        return self._request("DELETE", path, timeout=timeout, use_auth=use_auth)

    def get_raw(self, path, timeout=30, use_auth=True):
        """The status code alone, 0 when the request could not be made."""
        try:
            status, _ = self._raw("GET", path, timeout=timeout, use_auth=use_auth)
        except ConnectionError:
            return 0
        return status

    def get_status_and_body(self, path, timeout=30, use_auth=True, extra_headers=None):
        """Return (status_code, body_text) tuple; (0, "") when the request could not be made."""
        try:
            status, data = self._raw("GET", path, timeout=timeout, use_auth=use_auth, extra_headers=extra_headers)
        except ConnectionError:
            return 0, ""
        return status, data.decode(errors="replace")

    def ws_upgrade_status(self, path, timeout=5):
        """Send a WebSocket upgrade request, return the HTTP status code."""
        return self._transport.status_line(
            path,
            {
                "Connection": "Upgrade",
                "Upgrade": "websocket",
                "Sec-WebSocket-Version": "13",
                "Sec-WebSocket-Key": "dGhlIHNhbXBsZSBub25jZQ==",
            },
            timeout=timeout,
        )
