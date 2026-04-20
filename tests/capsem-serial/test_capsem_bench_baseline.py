"""Record in-VM capsem-bench output as a time-series baseline.

Provisions a fresh VM, runs `capsem-bench all`, pulls /tmp/capsem-benchmark.json
out via /exec, and archives it to benchmarks/capsem-bench/data_<version>_<arch>.json.

No gates yet -- we lack a stable baseline. Once 5-10 clean runs are on
disk per arch, per-category tolerances can be picked and promoted to
pytest asserts (mirroring OP_GATE_MS / FORK_GATE_MS in
test_lifecycle_benchmark.py).
"""

import json
import os
import re
import time
import uuid
from pathlib import Path

import pytest

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.service import ServiceInstance, wait_exec_ready

pytestmark = pytest.mark.serial

PROJECT_ROOT = Path(__file__).parent.parent.parent


def _project_version():
    cargo = PROJECT_ROOT / "Cargo.toml"
    m = re.search(r'^version\s*=\s*"([^"]+)"', cargo.read_text(), re.MULTILINE)
    return m.group(1) if m else "unknown"


def _save(data):
    version = _project_version()
    arch = "arm64" if os.uname().machine == "arm64" else "x86_64"
    out_dir = PROJECT_ROOT / "benchmarks" / "capsem-bench"
    out_dir.mkdir(parents=True, exist_ok=True)
    out_path = out_dir / f"data_{version}_{arch}.json"
    with open(out_path, "w") as f:
        json.dump(data, f, indent=2)
    print(f"capsem-bench baseline archived to {out_path}")


def test_capsem_bench_baseline():
    """Run capsem-bench all in a fresh VM, archive the JSON output."""
    svc = ServiceInstance()
    svc.start()
    client = svc.client()
    name = f"bench-{uuid.uuid4().hex[:8]}"

    try:
        client.post("/provision", {
            "name": name,
            "ram_mb": DEFAULT_RAM_MB,
            "cpus": DEFAULT_CPUS,
        })
        assert wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT), (
            f"{name} not ready"
        )

        # Full suite: disk, rootfs, startup, http, throughput, snapshot.
        # 10-minute cap covers the 256MB disk tests + 10MB download +
        # 50 HTTP requests + snapshot ops without false-timing.
        resp = client.post(
            f"/exec/{name}",
            {"command": "capsem-bench all", "timeout_secs": 600},
            timeout=610,
        )
        assert resp and resp.get("exit_code") == 0, (
            f"capsem-bench all failed: exit={resp.get('exit_code') if resp else None}\n"
            f"stdout: {(resp or {}).get('stdout', '')[:500]}\n"
            f"stderr: {(resp or {}).get('stderr', '')[:500]}"
        )

        # capsem-bench writes /tmp/capsem-benchmark.json on success (see
        # guest/artifacts/capsem_bench/__main__.py). Pull it out before
        # the VM is torn down.
        resp = client.post(
            f"/exec/{name}",
            {"command": "cat /tmp/capsem-benchmark.json", "timeout_secs": 15},
            timeout=20,
        )
        assert resp and resp.get("exit_code") == 0, (
            "capsem-bench did not produce /tmp/capsem-benchmark.json"
        )
        raw = resp.get("stdout", "").strip()
        data = json.loads(raw)
        # Stamp host-side metadata so a future comparison helper can group
        # by arch and time without re-reading Cargo.toml.
        data["host_recorded_at"] = time.time()
        data["arch"] = os.uname().machine
        _save(data)
    finally:
        try:
            client.delete(f"/delete/{name}")
        except Exception:
            pass
        svc.stop()
