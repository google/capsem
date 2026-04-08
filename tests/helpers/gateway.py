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

PROJECT_ROOT = Path(__file__).parent.parent.parent
GATEWAY_BINARY = PROJECT_ROOT / "target/debug/capsem-gateway"


class GatewayInstance:
    """A running capsem-gateway on an isolated temp dir."""

    def __init__(self, uds_path: str | Path, port: int = 0):
        self.tmp_dir = Path(tempfile.mkdtemp(prefix="capsem-gw-test-"))
        self.uds_path = str(uds_path)
        self._port = port
        self.proc = None
        self._log_file = None
        self.token = None
        self.port = None

    def start(self):
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

        log_path = self.tmp_dir / "gateway.log"
        print(f"GATEWAY LOG: {log_path}")
        self._log_file = open(log_path, "w")

        self.proc = subprocess.Popen(
            [
                str(GATEWAY_BINARY),
                "--port", str(self._port),
                "--uds-path", self.uds_path,
            ],
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
                try:
                    result = subprocess.run(
                        ["curl", "-s", "--max-time", "2",
                         f"http://127.0.0.1:{self.port}/"],
                        capture_output=True, text=True, timeout=5,
                    )
                    if result.returncode == 0 and "ok" in result.stdout.lower():
                        return
                except Exception:
                    pass
            time.sleep(0.2)

        self.stop()
        if log_path.exists():
            print(f"\n--- GATEWAY LOG ---\n{log_path.read_text()}\n---", file=sys.stderr)
        raise RuntimeError("capsem-gateway failed to start within 10s")

    def stop(self):
        if self.proc:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait()
        if self._log_file:
            self._log_file.close()

    @property
    def base_url(self) -> str:
        return f"http://127.0.0.1:{self.port}"

    @property
    def auth_header(self) -> str:
        return f"Bearer {self.token}"

    @property
    def run_dir(self) -> Path:
        return self.tmp_dir / ".capsem" / "run"


class TcpHttpClient:
    """HTTP client for talking to the gateway over TCP with auth."""

    def __init__(self, base_url: str, token: str):
        self.base_url = base_url
        self.token = token

    def _curl(self, method, path, body=None, timeout=30, use_auth=True):
        cmd = [
            "curl", "-s", "-S",
            "-X", method,
            "-H", "Content-Type: application/json",
            "--max-time", str(timeout),
        ]
        if use_auth:
            cmd += ["-H", f"Authorization: Bearer {self.token}"]
        if body is not None:
            cmd += ["-d", json.dumps(body)]
        cmd.append(f"{self.base_url}{path}")
        result = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout + 5)
        if result.returncode != 0:
            raise ConnectionError(f"curl failed (rc={result.returncode}): {result.stderr}")
        if not result.stdout.strip():
            return None
        return json.loads(result.stdout)

    def get(self, path, timeout=30, use_auth=True):
        return self._curl("GET", path, timeout=timeout, use_auth=use_auth)

    def post(self, path, body=None, timeout=60, use_auth=True):
        return self._curl("POST", path, body, timeout=timeout, use_auth=use_auth)

    def delete(self, path, timeout=30, use_auth=True):
        return self._curl("DELETE", path, timeout=timeout, use_auth=use_auth)

    def get_raw(self, path, timeout=30, use_auth=True):
        """Return raw curl output (status code + body) for status assertions."""
        cmd = [
            "curl", "-s", "-S",
            "-o", "/dev/null",
            "-w", "%{http_code}",
            "--max-time", str(timeout),
        ]
        if use_auth:
            cmd += ["-H", f"Authorization: Bearer {self.token}"]
        cmd.append(f"{self.base_url}{path}")
        result = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout + 5)
        return int(result.stdout.strip()) if result.stdout.strip() else 0
