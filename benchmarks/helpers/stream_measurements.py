"""Current service-owned stream endpoint and raw per-phase resource samples."""

from __future__ import annotations

import time
from typing import NamedTuple

from helpers.process_resources import ProcessTreeSampler


class Gateway(NamedTuple):
    port: int
    token: str
    pid: int


def service_gateway(service) -> Gateway:
    """Reuse the real service companion, never spawn a second gateway."""
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        try:
            token = (service.tmp_dir / "gateway.token").read_text().strip()
            port = (service.tmp_dir / "gateway.port").read_text().strip()
            pid = (service.tmp_dir / "gateway.pid").read_text().strip()
        except FileNotFoundError:
            token = port = pid = ""
        # Files are prepared empty before publish; nonempty token is last.
        if token and port and pid:
            return Gateway(int(port), token, int(pid))
        time.sleep(0.05)
    raise RuntimeError("service gateway did not publish its endpoint")


def measure(metrics: dict, phase: str, root, operation):
    """Record raw process-tree pressure beside a successful measured action."""
    sampler = ProcessTreeSampler(root)
    sampler.start()
    started = time.monotonic()
    try:
        return operation()
    finally:
        elapsed = time.monotonic() - started
        metrics.update({f"{phase}.{key}": value for key, value in sampler.stop().items()})
        metrics[f"{phase}.wall_s"] = {"unit": "seconds", "samples": [elapsed]}
