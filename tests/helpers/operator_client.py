"""Operator test traffic uses the actual authenticated service-owned gateway.

Delegated tests must pass their own credential to TcpHttpClient; this helper
intentionally reads the operator's private fixture runtime files.
"""

from __future__ import annotations

import time
from pathlib import Path

from .gateway import _runtime_connection
from .http_transport import Transport
from .uds_client import HttpClient


class OperatorServiceClient(HttpClient):
    def __init__(self, run_dir: Path, *, before_vm_delete=None):
        self.run_dir = run_dir
        self._connection = self._ready_connection()
        _, port = self._connection
        super().__init__(Transport(host="127.0.0.1", port=port), before_vm_delete=before_vm_delete)

    def _ready_connection(self):
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            if not self.run_dir.exists():
                raise ConnectionError("service fixture runtime is unavailable")
            connection = _runtime_connection(self.run_dir)
            if connection is not None:
                return connection
            time.sleep(0.05)
        raise ConnectionError("service fixture gateway did not publish authenticated readiness")

    def call(self, method, path, *, body=None, headers=None, timeout=60, translate=True):
        connection = self._ready_connection()
        token, port = connection
        if connection != self._connection:
            self._transport.close()
            self._transport = Transport(host="127.0.0.1", port=port)
            self._connection = connection
        headers = dict(headers or {})
        if not any(name.lower() == "authorization" for name in headers):
            headers["Authorization"] = f"Bearer {token}"
        return super().call(
            method, path, body=body, headers=headers, timeout=timeout, translate=translate
        )
