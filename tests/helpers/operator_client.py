"""Operator test traffic uses the actual authenticated service-owned gateway.

Delegated tests must pass their own credential to TcpHttpClient; this helper
intentionally reads the operator's private fixture runtime files.
"""

from __future__ import annotations

from pathlib import Path

from .http_transport import Transport
from .service_gateway import wait_for_connection
from .uds_client import HttpClient


class OperatorServiceClient(HttpClient):
    def __init__(self, run_dir: Path, *, before_vm_delete=None):
        self.run_dir = run_dir
        self._connection = self._ready_connection()
        _, port = self._connection
        super().__init__(Transport(host="127.0.0.1", port=port), before_vm_delete=before_vm_delete)

    def _ready_connection(self):
        return wait_for_connection(self.run_dir)

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
