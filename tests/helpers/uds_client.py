"""Minimal HTTP-over-UDS client for testing capsem-service directly."""

import json
import subprocess


class UdsHttpClient:
    """HTTP client that talks to an Axum server over a Unix Domain Socket via curl."""

    def __init__(self, socket_path):
        self.socket_path = str(socket_path)

    def _curl(self, method, path, body=None, timeout=60):
        cmd = [
            "curl", "-s", "-S",
            "--unix-socket", self.socket_path,
            "-X", method,
            "-H", "Content-Type: application/json",
            "--max-time", str(timeout),
            f"http://localhost{path}",
        ]
        if body is not None:
            cmd += ["-d", json.dumps(body)]
        result = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout + 5)
        if result.returncode != 0:
            raise ConnectionError(f"curl failed: {result.stderr}")
        if not result.stdout.strip():
            return None
        return json.loads(result.stdout)

    def post(self, path, body=None, timeout=60):
        return self._curl("POST", path, body, timeout)

    def get(self, path, timeout=60):
        return self._curl("GET", path, timeout=timeout)

    def get_text(self, path, timeout=60):
        """GET returning raw text (for endpoints that don't return JSON, e.g. /service-logs)."""
        cmd = [
            "curl", "-s", "-S",
            "--unix-socket", self.socket_path,
            "-X", "GET",
            "--max-time", str(timeout),
            f"http://localhost{path}",
        ]
        result = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout + 5)
        if result.returncode != 0:
            raise ConnectionError(f"curl failed: {result.stderr}")
        return result.stdout

    def delete(self, path, timeout=60):
        return self._curl("DELETE", path, timeout=timeout)
