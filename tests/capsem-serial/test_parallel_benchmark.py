"""Parallel VM benchmark test (host-side).

Spawns 4 VMs and runs capsem-bench in parallel in all of them to measure
performance degradation under heavy concurrent load.
"""

import json
import time
import uuid
from pathlib import Path
from concurrent.futures import ThreadPoolExecutor

import pytest

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.service import ServiceInstance, wait_exec_ready

pytestmark = [pytest.mark.serial, pytest.mark.benchmark]

PROJECT_ROOT = Path(__file__).parent.parent.parent
RUNS = 1  # Long lasting test, 1 run is enough for now
NUM_VMS = 4

def _save_benchmark(category, data):
    """Save benchmark JSON to benchmarks/{category}/data_{version}.json."""
    # Minimal version reading for now
    version = "1.0"
    out_dir = PROJECT_ROOT / "benchmarks" / category
    out_dir.mkdir(parents=True, exist_ok=True)
    out_path = out_dir / f"data_{version}.json"
    with open(out_path, "w") as f:
        json.dump(data, f, indent=2)
    print(f"Benchmark saved to {out_path}")

def _run_benchmark_in_vm(client, vm_name):
    """Run capsem-bench all in the VM and return the output."""
    print(f"Starting benchmark in {vm_name}...")
    t0 = time.monotonic()
    # capsem-bench all might take ~2 min, so set a large timeout
    resp = client.post(
        f"/exec/{vm_name}",
        {"command": "capsem-bench all", "timeout_secs": 300},
        timeout=310,
    )
    duration_ms = (time.monotonic() - t0) * 1000
    
    if resp is None or resp.get("exit_code") != 0:
        print(f"Benchmark failed in {vm_name}: {resp}")
        return {"vm": vm_name, "status": "failed", "duration_ms": duration_ms}
    
    print(f"Benchmark completed in {vm_name} in {duration_ms:.0f}ms")
    return {"vm": vm_name, "status": "success", "duration_ms": duration_ms, "stdout": resp.get("stdout")}

def test_parallel_benchmark():
    """Spawn 4 VMs and run benchmarks in parallel."""
    svc = ServiceInstance()
    svc.start()
    client = svc.client()

    vms = [f"par-bench-{uuid.uuid4().hex[:6]}-{i}" for i in range(NUM_VMS)]
    
    try:
        # 1. Spawn VMs sequentially (to separate spawning from execution contention)
        print(f"Spawning {NUM_VMS} VMs...")
        for vm_name in vms:
            client.post("/provision", {"name": vm_name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})
            assert wait_exec_ready(client, vm_name, timeout=EXEC_READY_TIMEOUT), f"{vm_name} not ready"
            print(f"VM {vm_name} spawned and ready.")

        # 2. Run benchmarks in parallel
        print(f"Running benchmarks in parallel in {NUM_VMS} VMs...")
        t0 = time.monotonic()
        with ThreadPoolExecutor(max_workers=NUM_VMS) as executor:
            futures = [executor.submit(_run_benchmark_in_vm, client, vm_name) for vm_name in vms]
            results = [f.result() for f in futures]
        total_duration_ms = (time.monotonic() - t0) * 1000

        print(f"All parallel benchmarks completed in {total_duration_ms:.0f}ms")

        # 3. Report results
        summary = {
            "version": "1.0",
            "timestamp": time.time(),
            "num_vms": NUM_VMS,
            "total_duration_ms": total_duration_ms,
            "results": results,
        }
        
        _save_benchmark("parallel", summary)

        # Check if any failed
        failed = [r for r in results if r["status"] != "success"]
        assert not failed, f"Some benchmarks failed: {failed}"

    finally:
        print("Cleaning up VMs...")
        for vm_name in vms:
            try:
                client.delete(f"/delete/{vm_name}")
            except Exception:
                pass
        svc.stop()
