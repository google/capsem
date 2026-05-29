"""Host-native benchmark baseline.

This records the host-side baseline in the same artifact stream as the VM
benchmarks so VM results can be compared against the hardware that produced
them.
"""

import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

import pytest

from helpers.benchmark_artifacts import (
    benchmark_arch,
    benchmark_output_path,
    enrich_benchmark_artifact,
)

pytestmark = [pytest.mark.serial, pytest.mark.benchmark]

PROJECT_ROOT = Path(__file__).parent.parent.parent
GUEST_ARTIFACTS = PROJECT_ROOT / "guest" / "artifacts"
if str(GUEST_ARTIFACTS) not in sys.path:
    sys.path.insert(0, str(GUEST_ARTIFACTS))

from capsem_bench.disk import disk_bench  # noqa: E402
from capsem_bench.helpers import BLOCK_4K, throughput_mbps  # noqa: E402
from capsem_bench.startup import startup_bench  # noqa: E402


def _project_version():
    cargo = PROJECT_ROOT / "Cargo.toml"
    m = re.search(r'^version\s*=\s*"([^"]+)"', cargo.read_text(), re.MULTILINE)
    return m.group(1) if m else "unknown"


def _save(data):
    version = _project_version()
    arch = benchmark_arch()
    out_path = benchmark_output_path(PROJECT_ROOT, "host-native", version, arch)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    data = enrich_benchmark_artifact(
        data,
        project_root=PROJECT_ROOT,
        project_version=version,
        arch=arch,
        command="uv run pytest tests/capsem-serial/test_host_native_benchmark.py -xvs",
    )
    with open(out_path, "w") as f:
        json.dump(data, f, indent=2)
    print(f"Host-native benchmark saved to {out_path}")


def _default_bench_dir() -> Path:
    path = PROJECT_ROOT / "target" / "host-native-benchmark"
    path.mkdir(parents=True, exist_ok=True)
    return path


def _df_context(directory: Path) -> dict:
    context = {
        "directory": str(directory),
        "disk_usage": {
            "total_bytes": shutil.disk_usage(directory).total,
            "used_bytes": shutil.disk_usage(directory).used,
            "free_bytes": shutil.disk_usage(directory).free,
        },
    }
    try:
        output = subprocess.check_output(
            ["df", "-PT", str(directory)],
            text=True,
            stderr=subprocess.DEVNULL,
        ).splitlines()
    except Exception:
        return context

    if len(output) >= 2:
        fields = output[1].split()
        if len(fields) >= 7:
            context["df"] = {
                "source": fields[0],
                "fstype": fields[1],
                "blocks_1k": int(fields[2]),
                "used_1k": int(fields[3]),
                "available_1k": int(fields[4]),
                "capacity": fields[5],
                "mount": " ".join(fields[6:]),
            }
    return context


def _small_file_read_bench(directory: Path) -> dict:
    files_dir = directory / "small-files"
    files_dir.mkdir()
    files = []
    payload = b"export const value = 'capsem benchmark';\n" * 16
    for idx in range(128):
        path = files_dir / f"module-{idx:03d}.js"
        path.write_bytes(payload)
        files.append(path)

    reads = int(os.environ.get("CAPSEM_HOST_NATIVE_SMALL_READS", "5000"))
    start = time.monotonic()
    total = 0
    for idx in range(reads):
        total += len(files[idx % len(files)].read_bytes())
    elapsed = time.monotonic() - start
    return {
        "count": reads,
        "files_sampled": len(files),
        "bytes_read": total,
        "duration_ms": round(elapsed * 1000, 1),
        "ops_per_sec": round(reads / elapsed, 1) if elapsed > 0 else 0.0,
        "throughput_mbps": throughput_mbps(total, elapsed),
    }


def _metadata_stat_bench(directory: Path) -> dict:
    tree = directory / "metadata-tree"
    entries = int(os.environ.get("CAPSEM_HOST_NATIVE_STAT_ENTRIES", "5000"))
    for idx in range(entries):
        subdir = tree / f"d{idx // 100:03d}"
        subdir.mkdir(parents=True, exist_ok=True)
        (subdir / f"f{idx:05d}.txt").write_text("capsem\n")

    paths = list(tree.rglob("*"))
    start = time.monotonic()
    files = dirs = errors = 0
    for path in paths:
        try:
            stat = path.lstat()
        except OSError:
            errors += 1
            continue
        if path.is_dir():
            dirs += 1
        elif stat:
            files += 1
    elapsed = time.monotonic() - start
    return {
        "entries": len(paths),
        "files": files,
        "dirs": dirs,
        "errors": errors,
        "duration_ms": round(elapsed * 1000, 1),
        "stats_per_sec": round(len(paths) / elapsed, 1) if elapsed > 0 else 0.0,
    }


def test_host_native_benchmark():
    size_mb = int(
        os.environ.get(
            "CAPSEM_HOST_NATIVE_BENCH_SIZE_MB",
            os.environ.get("CAPSEM_BENCH_SIZE_MB", "256"),
        )
    )
    base_dir_env = os.environ.get("CAPSEM_HOST_NATIVE_BENCH_DIR")
    base_dir = Path(base_dir_env) if base_dir_env else _default_bench_dir()
    base_dir.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=base_dir) as tmp:
        bench_dir = Path(tmp)
        data = {
            "kind": "host_native_baseline",
            "version": "0.1.0",
            "timestamp": time.time(),
            "filesystem": _df_context(bench_dir),
            "disk": disk_bench(str(bench_dir), size_mb=size_mb),
            "startup": startup_bench(),
            "small_file_read": _small_file_read_bench(bench_dir),
            "metadata_stat": _metadata_stat_bench(bench_dir),
            "io_shape": {
                "sequential_block_size": 1024 * 1024,
                "random_block_size": BLOCK_4K,
                "size_mb": size_mb,
            },
        }
    _save(data)

    assert data["disk"]["seq_read"]["throughput_mbps"] > 0
    assert data["disk"]["rand_read_4k"]["iops"] > 0
    assert data["metadata_stat"]["stats_per_sec"] > 0
