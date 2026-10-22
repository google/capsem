"""Current service-owned stream endpoint and raw per-phase resource samples."""

from __future__ import annotations

import time
from typing import NamedTuple

from helpers.process_resources import ProcessTreeSampler


class Gateway(NamedTuple):
    port: int
    token: str


def service_gateway(service) -> Gateway:
    """Reuse the real service companion, never spawn a second gateway."""
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        port = service.tmp_dir / "gateway.port"
        token = service.tmp_dir / "gateway.token"
        if port.exists() and token.exists():
            return Gateway(int(port.read_text()), token.read_text().strip())
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
