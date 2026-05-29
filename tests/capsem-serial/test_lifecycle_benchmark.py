"""VM lifecycle operation benchmarks (host-side).

Profiles individual operations: provision, exec-ready wait, exec, delete,
fork, boot-from-image. Reports per-operation timings as a Rich table + JSON.

Fork gates: fork < 500ms, image size < 128MB, boot-from-image verifies data.
"""

import json
import os
import platform
import re
import time
import uuid
from pathlib import Path

import pytest

from helpers.benchmark_artifacts import (
    benchmark_arch,
    benchmark_output_path,
    enrich_benchmark_artifact,
)
from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.service import ServiceInstance, wait_exec_ready

pytestmark = [pytest.mark.serial, pytest.mark.benchmark]

PROJECT_ROOT = Path(__file__).parent.parent.parent


def _project_version():
    """Read version from workspace Cargo.toml."""
    cargo = PROJECT_ROOT / "Cargo.toml"
    m = re.search(r'^version\s*=\s*"([^"]+)"', cargo.read_text(), re.MULTILINE)
    return m.group(1) if m else "unknown"


def _save_benchmark(category, data, command):
    """Save benchmark JSON to an arch-scoped artifact path."""
    version = _project_version()
    arch = benchmark_arch()
    out_path = benchmark_output_path(PROJECT_ROOT, category, version, arch)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    data = enrich_benchmark_artifact(
        data,
        project_root=PROJECT_ROOT,
        project_version=version,
        arch=arch,
        command=command,
    )
    with open(out_path, "w") as f:
        json.dump(data, f, indent=2)
    print(f"Benchmark saved to {out_path}")

RUNS = 3
PROVISION_GATE_MS = 3500 if platform.system() == "Linux" else 1200
OP_GATE_MS = 1200  # steady-state operations must complete under this
FORK_GATE_MS = 500
# The fork workload intentionally runs apt-get update/install, so current
# package-manager metadata plus the installed jq overlay lands around 105MB on
# the Linux KVM x86_64 image even after cleaning transient apt index/cache data.
# Keep the gate workload-aware while still far below a sparse 2GB logical-size
# regression; the lower-level disk-usage unit test covers sparse accounting.
IMAGE_SIZE_GATE_MB = 128


def _gate_env_ms(name, default):
    try:
        return float(os.environ.get(name, default))
    except ValueError:
        return default


def _run_lifecycle(client):
    """Run one full provision -> exec-ready -> exec -> delete cycle.

    Returns dict with per-operation timings in ms.
    """
    name = f"bench-{uuid.uuid4().hex[:8]}"

    t0 = time.monotonic()
    client.post("/provision", {"name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})
    provision_ms = (time.monotonic() - t0) * 1000

    t0 = time.monotonic()
    ready = wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)
    exec_ready_ms = (time.monotonic() - t0) * 1000
    assert ready, f"VM {name} never became exec-ready"

    t0 = time.monotonic()
    resp = client.post(f"/exec/{name}", {"command": "echo ok", "timeout_secs": 10}, timeout=15)
    exec_ms = (time.monotonic() - t0) * 1000
    assert resp is not None and "ok" in resp.get("stdout", "")

    t0 = time.monotonic()
    client.delete(f"/delete/{name}")
    delete_ms = (time.monotonic() - t0) * 1000

    return {
        "name": name,
        "provision_ms": round(provision_ms, 1),
        "exec_ready_ms": round(exec_ready_ms, 1),
        "exec_ms": round(exec_ms, 1),
        "delete_ms": round(delete_ms, 1),
    }


def _run_fork_benchmark(client):
    """Provision VM -> install packages -> write workspace -> fork -> verify.

    Returns dict with fork timing, image size, and boot-from-image timing.
    """
    src = f"fkb-{uuid.uuid4().hex[:6]}"
    img = f"fki-{uuid.uuid4().hex[:6]}"
    dst = f"fkd-{uuid.uuid4().hex[:6]}"

    try:
        # Provision source VM and wait for exec
        client.post("/provision", {"name": src, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})
        assert wait_exec_ready(client, src, timeout=EXEC_READY_TIMEOUT), f"{src} not ready"

        # Install a package (rootfs overlay change)
        resp = client.post(f"/exec/{src}", {
            "command": (
                "apt-get update -qq && "
                "apt-get install -y -qq jq && "
                "rm -rf /var/lib/apt/lists/* /var/cache/apt/archives/*.deb"
            ),
            "timeout_secs": 120,
        }, timeout=130)
        assert resp and resp.get("exit_code") == 0, f"apt-get failed: {resp}"

        # Write workspace file
        client.write_file(src, "/root/bench.txt", "fork-benchmark-marker")

        # Fork -- time it
        t0 = time.monotonic()
        fork_resp = client.post(f"/fork/{src}", {"name": img})
        fork_ms = (time.monotonic() - t0) * 1000

        size_bytes = fork_resp.get("size_bytes", 0)
        size_mb = size_bytes / (1024 * 1024)

        # Boot from fork -- time provision + exec-ready
        t0 = time.monotonic()
        client.post("/provision", {
            "name": dst, "from": img,
            "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS,
        })
        boot_provision_ms = (time.monotonic() - t0) * 1000

        t0 = time.monotonic()
        assert wait_exec_ready(client, dst, timeout=EXEC_READY_TIMEOUT), f"{dst} not ready"
        boot_ready_ms = (time.monotonic() - t0) * 1000

        # Verify packages survived (rootfs overlay)
        resp = client.post(f"/exec/{dst}", {"command": "which jq", "timeout_secs": 10}, timeout=15)
        pkg_survived = resp is not None and resp.get("exit_code") == 0

        # Verify workspace survived
        resp = client.post(f"/exec/{dst}", {
            "command": "cat /root/bench.txt", "timeout_secs": 10,
        }, timeout=15)
        ws_survived = resp is not None and "fork-benchmark-marker" in resp.get("stdout", "")

        return {
            "fork_ms": round(fork_ms, 1),
            "image_size_mb": round(size_mb, 2),
            "boot_provision_ms": round(boot_provision_ms, 1),
            "boot_ready_ms": round(boot_ready_ms, 1),
            "pkg_survived": pkg_survived,
            "ws_survived": ws_survived,
        }
    finally:
        for v in [dst, src, img]:
            try:
                client.delete(f"/delete/{v}")
            except Exception:
                pass


def test_lifecycle_benchmark():
    """Profile VM lifecycle operations over 3 runs; print Rich table + JSON."""
    svc = ServiceInstance()
    svc.start()
    client = svc.client()

    runs = []
    try:
        for i in range(RUNS):
            result = _run_lifecycle(client)
            runs.append(result)
            total = (
                result["provision_ms"]
                + result["exec_ready_ms"]
                + result["exec_ms"]
                + result["delete_ms"]
            )
            print(
                f"  run {i+1}: provision={result['provision_ms']:.0f}ms"
                f"  exec_ready={result['exec_ready_ms']:.0f}ms"
                f"  exec={result['exec_ms']:.0f}ms"
                f"  delete={result['delete_ms']:.0f}ms"
                f"  total={total:.0f}ms"
            )
    finally:
        svc.stop()

    def avg(key):
        return round(sum(r[key] for r in runs) / len(runs), 1)

    def mn(key):
        return round(min(r[key] for r in runs), 1)

    def mx(key):
        return round(max(r[key] for r in runs), 1)

    summary = {
        "version": "0.1.0",
        "timestamp": time.time(),
        "runs": RUNS,
        "operations": {},
    }
    for op in ("provision_ms", "exec_ready_ms", "exec_ms", "delete_ms"):
        summary["operations"][op] = {
            "min": mn(op),
            "mean": avg(op),
            "max": mx(op),
            "values": [r[op] for r in runs],
        }

    total_values = [
        r["provision_ms"] + r["exec_ready_ms"] + r["exec_ms"] + r["delete_ms"]
        for r in runs
    ]
    summary["operations"]["total_ms"] = {
        "min": round(min(total_values), 1),
        "mean": round(sum(total_values) / len(total_values), 1),
        "max": round(max(total_values), 1),
        "values": [round(v, 1) for v in total_values],
    }

    # Rich table
    print()
    print(f"VM Lifecycle Benchmark  [{RUNS} runs]")
    print(f"{'Operation':<16} {'Min':>10} {'Mean':>10} {'Max':>10}")
    print("-" * 50)
    for op, label in [
        ("provision_ms", "provision"),
        ("exec_ready_ms", "exec_ready"),
        ("exec_ms", "exec"),
        ("delete_ms", "delete"),
        ("total_ms", "TOTAL"),
    ]:
        s = summary["operations"][op]
        print(f"{label:<16} {s['min']:>9.0f}ms {s['mean']:>9.0f}ms {s['max']:>9.0f}ms")

    # JSON output
    _save_benchmark(
        "lifecycle",
        summary,
        "uv run pytest tests/capsem-serial/test_lifecycle_benchmark.py::test_lifecycle_benchmark -xvs",
    )

    # Gate: provision is host/backend dependent; steady-state ops stay tight.
    gates = {
        "provision_ms": _gate_env_ms("CAPSEM_PROVISION_GATE_MS", PROVISION_GATE_MS),
        "exec_ready_ms": _gate_env_ms("CAPSEM_EXEC_READY_GATE_MS", OP_GATE_MS),
        "exec_ms": _gate_env_ms("CAPSEM_EXEC_GATE_MS", OP_GATE_MS),
        "delete_ms": _gate_env_ms("CAPSEM_DELETE_GATE_MS", OP_GATE_MS),
    }
    for op, label in [
        ("provision_ms", "provision"),
        ("exec_ready_ms", "exec_ready"),
        ("exec_ms", "exec"),
        ("delete_ms", "delete"),
    ]:
        mean = summary["operations"][op]["mean"]
        gate = gates[op]
        assert mean < gate, (
            f"{label} mean {mean:.0f}ms exceeds {gate:.0f}ms gate"
        )


def test_fork_benchmark():
    """Profile fork: speed, image size, data survival. Regression gates."""
    svc = ServiceInstance()
    svc.start()
    client = svc.client()

    runs = []
    try:
        for i in range(RUNS):
            result = _run_fork_benchmark(client)
            runs.append(result)
            print(
                f"  run {i+1}: fork={result['fork_ms']:.0f}ms"
                f"  size={result['image_size_mb']:.1f}MB"
                f"  boot={result['boot_provision_ms']:.0f}ms"
                f"  ready={result['boot_ready_ms']:.0f}ms"
                f"  pkg={'ok' if result['pkg_survived'] else 'FAIL'}"
                f"  ws={'ok' if result['ws_survived'] else 'FAIL'}"
            )
    finally:
        svc.stop()

    def avg(key):
        return round(sum(r[key] for r in runs) / len(runs), 1)

    def mn(key):
        return round(min(r[key] for r in runs), 1)

    def mx(key):
        return round(max(r[key] for r in runs), 1)

    summary = {
        "version": "0.1.0",
        "timestamp": time.time(),
        "runs": RUNS,
        "fork": {},
    }
    for op in ("fork_ms", "image_size_mb", "boot_provision_ms", "boot_ready_ms"):
        summary["fork"][op] = {
            "min": mn(op),
            "mean": avg(op),
            "max": mx(op),
            "values": [r[op] for r in runs],
        }

    # Rich table
    print()
    print(f"Fork Benchmark  [{RUNS} runs]")
    print(f"{'Metric':<20} {'Min':>10} {'Mean':>10} {'Max':>10} {'Gate':>10}")
    print("-" * 65)
    s = summary["fork"]["fork_ms"]
    print(f"{'fork':<20} {s['min']:>9.0f}ms {s['mean']:>9.0f}ms {s['max']:>9.0f}ms {FORK_GATE_MS:>9}ms")
    s = summary["fork"]["image_size_mb"]
    image_size_gate_mb = _gate_env_ms("CAPSEM_FORK_IMAGE_SIZE_GATE_MB", IMAGE_SIZE_GATE_MB)
    print(f"{'image_size':<20} {s['min']:>9.1f}MB {s['mean']:>9.1f}MB {s['max']:>9.1f}MB {image_size_gate_mb:>9.0f}MB")
    s = summary["fork"]["boot_provision_ms"]
    boot_gate_ms = _gate_env_ms("CAPSEM_FORK_BOOT_PROVISION_GATE_MS", PROVISION_GATE_MS)
    print(f"{'boot_provision':<20} {s['min']:>9.0f}ms {s['mean']:>9.0f}ms {s['max']:>9.0f}ms {boot_gate_ms:>9.0f}ms")
    s = summary["fork"]["boot_ready_ms"]
    print(f"{'boot_ready':<20} {s['min']:>9.0f}ms {s['mean']:>9.0f}ms {s['max']:>9.0f}ms {OP_GATE_MS:>9}ms")

    # JSON output
    _save_benchmark(
        "fork",
        summary,
        "uv run pytest tests/capsem-serial/test_lifecycle_benchmark.py::test_fork_benchmark -xvs",
    )

    # Gate: fork speed
    fork_mean = summary["fork"]["fork_ms"]["mean"]
    assert fork_mean < FORK_GATE_MS, (
        f"fork mean {fork_mean:.0f}ms exceeds {FORK_GATE_MS}ms gate"
    )

    # Gate: image size (not a bloated 2GB sparse lie)
    size_max = summary["fork"]["image_size_mb"]["max"]
    assert size_max < image_size_gate_mb, (
        f"image size {size_max:.1f}MB exceeds {image_size_gate_mb:.0f}MB gate"
    )

    # Gate: boot-from-image speed
    boot_mean = summary["fork"]["boot_provision_ms"]["mean"]
    assert boot_mean < boot_gate_ms, (
        f"boot_provision mean {boot_mean:.0f}ms exceeds {boot_gate_ms:.0f}ms gate"
    )

    # Gate: data survival (every run must preserve both rootfs and workspace)
    for i, r in enumerate(runs):
        assert r["pkg_survived"], f"run {i+1}: packages did not survive fork"
        assert r["ws_survived"], f"run {i+1}: workspace files did not survive fork"
