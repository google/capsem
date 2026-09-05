"""Minimal HTTP-over-UDS client for testing capsem-service directly."""

import json
import uuid

from helpers.constants import CODE_PROFILE_ID
from helpers.http_transport import Transport


def _decode_payload(data: bytes):
    text = data.decode(errors="replace")
    if not text.strip():
        return None
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        return text


class UdsHttpClient:
    """HTTP client that talks to an Axum server over a Unix Domain Socket."""

    def __init__(self, socket_path, *, before_vm_delete=None):
        self.socket_path = str(socket_path)
        self._before_vm_delete = before_vm_delete
        self._transport = Transport(socket_path=self.socket_path)

    def _is_uuid(self, value):
        try:
            uuid.UUID(value)
            return True
        except ValueError:
            return False

    def _resolve_vm_path(self, path, timeout):
        prefix = "/vms/"
        if not path.startswith(prefix):
            return path
        rest = path[len(prefix):]
        segment, sep, suffix = rest.partition("/")
        if segment in {"create", "list"} or self._is_uuid(segment):
            return path
        try:
            listing = self._request("GET", "/vms/list", timeout=timeout, translate=False)
        except Exception:
            return path
        for row in listing.get("sandboxes", []):
            if row.get("id") == segment or row.get("name") == segment:
                resolved = row["id"]
                return f"{prefix}{resolved}{sep}{suffix}" if sep else f"{prefix}{resolved}"
        return path

    def call(self, method, path, *, body=None, headers=None, timeout=60, translate=True):
        """One request: (status, lower-cased response headers, body bytes).

        A 4xx or 5xx is a response, not an error; only a transport failure
        raises. `body` is sent as given (bytes) with `Content-Type` from
        `headers`, JSON by default.
        """
        if translate:
            path = self._resolve_vm_path(path, timeout)
        return self._transport.request(
            method,
            path,
            headers={"Content-Type": "application/json", **(headers or {})},
            body=body,
            timeout=timeout,
        )

    def call_json(self, method, path, body: object = None, *, timeout=60):
        """(status, payload): JSON when the body parses, the text when it does
        not, None when it is empty."""
        payload = None if body is None else json.dumps(body).encode()
        status, _, data = self.call(method, path, body=payload, timeout=timeout)
        return status, _decode_payload(data)

    def _raw(self, method, path, body=None, content_type="application/json", timeout=60, translate=True):
        status, _, data = self.call(
            method, path, body=body, headers={"Content-Type": content_type}, timeout=timeout, translate=translate
        )
        return status, data

    def _request(self, method, path, body=None, timeout=60, translate=True):
        """JSON in, JSON out (None for an empty body), whatever the status."""
        payload = None if body is None else json.dumps(body).encode()
        _, data = self._raw(method, path, payload, timeout=timeout, translate=translate)
        if not data.strip():
            return None
        return json.loads(data)

    def post(self, path, body=None, timeout=60):
        if path == "/vms/create" and isinstance(body, dict) and "profile_id" not in body:
            body = {**body, "profile_id": CODE_PROFILE_ID}
        return self._request("POST", path, body, timeout)

    def patch(self, path, body=None, timeout=60):
        return self._request("PATCH", path, body, timeout)

    def put(self, path, body=None, timeout=60):
        return self._request("PUT", path, body, timeout)

    def get(self, path, timeout=60):
        return self._request("GET", path, timeout=timeout)

    def get_text(self, path, timeout=60):
        """GET returning raw text (for endpoints that don't return JSON, e.g. /service-logs)."""
        _, data = self._raw("GET", path, timeout=timeout)
        return data.decode(errors="replace")

    def delete(self, path, timeout=60):
        if (
            self._before_vm_delete is not None
            and path.startswith("/vms/")
            and path.endswith("/delete")
        ):
            self._before_vm_delete()
        return self._request("DELETE", path, timeout=timeout)

    def post_bytes(self, path, data, timeout=60):
        """POST with a raw bytes body (for /vms/{id}/files/content uploads). Returns parsed JSON."""
        _, out = self._raw("POST", path, data, content_type="application/octet-stream", timeout=timeout)
        if not out.strip():
            return None
        return json.loads(out)

    def get_bytes(self, path, timeout=60):
        """GET returning raw bytes and status code (for binary downloads). Returns (status, body)."""
        return self._raw("GET", path, timeout=timeout)
