"""Explicit operator access to the real coordinator-owned fixture gateway.

Mock-service tests retain GatewayInstance. This view creates no broker or
process, and only ServiceInstance owns shutdown of the actual gateway.
"""

from __future__ import annotations

import os
import time
from pathlib import Path

import psutil
from log_streams import read_log_stream

from .gateway import GATEWAY_BINARY, _runtime_connection


def wait_for_connection(run_dir: Path) -> tuple[str, int]:
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        if not run_dir.is_dir():
            raise ConnectionError("service fixture runtime is unavailable")
        connection = _runtime_connection(run_dir)
        if connection is not None:
            return connection
        time.sleep(0.05)
    raise ConnectionError("service-owned gateway did not publish readiness")


class ServiceGateway:
    def __init__(self, service):
        self.service = service
        self.run_dir = service.tmp_dir
        self.proc = None
        self.port = None
        self.token = None
        self._log_path = self.run_dir / "gateway.log"

    def start(self):
        if self.service.proc is None or self.service.proc.poll() is not None:
            raise ConnectionError("gateway view requires its live service owner")
        token, port = wait_for_connection(self.run_dir)
        process = psutil.Process(int((self.run_dir / "gateway.pid").read_text()))
        if (
            process.ppid() != self.service.proc.pid
            or process.uids().effective != os.geteuid()
            or Path(process.exe()).resolve() != GATEWAY_BINARY.resolve()
        ):
            raise ConnectionError("gateway runtime does not belong to this service fixture")
        self.proc = process
        self.token, self.port = token, port
        print(f"GATEWAY LOG: {self._log_path}")

    @property
    def base_url(self):
        assert self.port is not None
        return f"http://127.0.0.1:{self.port}"

    def stop(self):
        """Detach the view; the service remains the sole process owner."""
        self.proc = None

    def stop_and_read_log(self):
        """Read complete evidence only after the service owner has drained it."""
        assert self.service.proc is None, "stop_and_read_log on the service owner must run first"
        if self.proc is not None:
            self.proc.wait(timeout=15)
        self.stop()
        return read_log_stream(self._log_path)
