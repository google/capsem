"""Minimal HTTP-over-UDS client for testing capsem-service directly."""

import json
import subprocess
from urllib.parse import quote


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

    @staticmethod
    def _sanitize_path(raw):
        cleaned = "".join(
            ch for ch in raw
            if ch.isascii() and (ch.isalnum() or ch in "._-/")
        )
        while "//" in cleaned:
            cleaned = cleaned.replace("//", "/")
        cleaned = cleaned.lstrip("/")
        if not cleaned:
            raise ValueError("empty path after sanitization")
        if ".." in cleaned:
            raise ValueError("path traversal rejected")
        return cleaned

    @classmethod
    def _files_content_path(cls, vm_id, path):
        sanitized = cls._sanitize_path(path)
        encoded = quote(sanitized, safe="")
        return f"/files/{vm_id}/content?path={encoded}"

    def write_file(self, vm_id, path, content, timeout=60):
        """Write text/bytes to VM workspace via canonical files endpoint."""
        endpoint = self._files_content_path(vm_id, path)
        data = content.encode("utf-8") if isinstance(content, str) else bytes(content)
        return self.post_bytes(endpoint, data, timeout=timeout)

    def read_file(self, vm_id, path, timeout=60):
        """Read text file from VM workspace via canonical files endpoint.

        Returns {"content": "..."} on success, or {"error": "..."} on failure.
        """
        endpoint = self._files_content_path(vm_id, path)
        status, body = self.get_bytes(endpoint, timeout=timeout)
        if status is None:
            return None
        if 200 <= status < 300:
            return {"content": body.decode("utf-8", errors="replace")}
        try:
            parsed = json.loads(body.decode("utf-8", errors="replace"))
            if isinstance(parsed, dict):
                return parsed
        except Exception:
            pass
        return {"error": body.decode("utf-8", errors="replace")}
