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

    def post_bytes(self, path, data, timeout=60):
        """POST with a raw bytes body (for /files/{id}/content uploads). Returns parsed JSON."""
        cmd = [
            "curl", "-s", "-S",
            "--unix-socket", self.socket_path,
            "-X", "POST",
            "-H", "Content-Type: application/octet-stream",
            "--max-time", str(timeout),
            "--data-binary", "@-",
            f"http://localhost{path}",
        ]
        result = subprocess.run(cmd, input=data, capture_output=True, timeout=timeout + 5)
        if result.returncode != 0:
            raise ConnectionError(f"curl failed: {result.stderr.decode(errors='replace')}")
        if not result.stdout.strip():
            return None
        return json.loads(result.stdout)

    def get_bytes(self, path, timeout=60):
        """GET returning raw bytes and status code (for binary downloads). Returns (status, body)."""
        cmd = [
            "curl", "-s", "-S", "-o", "-", "-w", "\n__STATUS__%{http_code}",
            "--unix-socket", self.socket_path,
            "-X", "GET",
            "--max-time", str(timeout),
            f"http://localhost{path}",
        ]
        result = subprocess.run(cmd, capture_output=True, timeout=timeout + 5)
        if result.returncode != 0:
            raise ConnectionError(f"curl failed: {result.stderr.decode(errors='replace')}")
        raw = result.stdout
        sep = b"\n__STATUS__"
        idx = raw.rfind(sep)
        if idx == -1:
            return None, raw
        status = int(raw[idx + len(sep):].decode(errors="replace"))
        body = raw[:idx]
        return status, body
