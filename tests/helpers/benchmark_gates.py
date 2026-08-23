"""Gross-regression gates for benchmark JSON artifacts."""

from __future__ import annotations

import sys
import tomllib
from pathlib import Path
from typing import Any, cast

PROJECT_ROOT = Path(__file__).resolve().parents[2]


def _thresholds() -> dict[str, Any]:
    config = tomllib.loads(
        (PROJECT_ROOT / "config" / "gate.toml").read_text(encoding="utf-8")
    )
    return config["benchmark"]["gates"]

#: Gross-regression floors and ceilings, read from `[benchmark.gates]` in
#: `config/gate.toml`. They were eleven literals here, which made this the one
#: place a number the gate judges by could be changed by someone who happened
#: to know the helper existed. Nothing is defaulted: a key config stops
#: declaring must fail loudly rather than compare against `None`.
CAPSEM_BENCH_GATES = _thresholds()


def validate_capsem_bench_result(data: dict[str, Any]) -> None:
    disk = data["disk"]
    disk_rand_iops_gate = _disk_rand_iops_gate()
    _assert_gte(
        disk["seq_write"]["throughput_mbps"],
        CAPSEM_BENCH_GATES["disk_seq_mbps"],
        "disk seq_write throughput",
    )
    _assert_gte(
        disk["seq_read"]["throughput_mbps"],
        CAPSEM_BENCH_GATES["disk_seq_mbps"],
        "disk seq_read throughput",
    )
    _assert_gte(
        disk["rand_write_4k"]["iops"],
        disk_rand_iops_gate,
        "disk rand_write_4k IOPS",
    )
    _assert_gte(
        disk["rand_read_4k"]["iops"],
        disk_rand_iops_gate,
        "disk rand_read_4k IOPS",
    )

    rootfs = data["rootfs"]
    _assert_gte(
        rootfs["seq_read"]["throughput_mbps"],
        CAPSEM_BENCH_GATES["rootfs_seq_mbps"],
        "rootfs seq_read throughput",
    )
    _assert_gte(
        rootfs["rand_read_4k"]["iops"],
        CAPSEM_BENCH_GATES["rootfs_rand_iops"],
        "rootfs rand_read_4k IOPS",
    )

    startup = data["startup"]["commands"]
    for command, gate_ms in CAPSEM_BENCH_GATES["startup_mean_ms"].items():
        _assert_lte(startup[command]["mean_ms"], gate_ms, f"startup {command} mean")

    http = data["http"]
    assert http["failed"] == 0, f"HTTP failed requests = {http['failed']}"
    assert http["successful"] == http["total_requests"], (
        f"HTTP successful {http['successful']} != total {http['total_requests']}"
    )
    _assert_gte(
        http["requests_per_sec"],
        CAPSEM_BENCH_GATES["http_min_rps"],
        "HTTP requests/sec",
    )
    _assert_lte(
        http["latency_ms"]["p99"],
        CAPSEM_BENCH_GATES["http_p99_ms"],
        "HTTP p99 latency",
    )

    throughput = data["throughput"]
    assert throughput["http_code"] == 200, (
        f"throughput HTTP code = {throughput['http_code']}"
    )
    _assert_gte(
        throughput["size_bytes"],
        CAPSEM_BENCH_GATES["throughput_min_bytes"],
        "throughput downloaded bytes",
    )
    _assert_gte(
        throughput["throughput_mbps"],
        CAPSEM_BENCH_GATES["throughput_min_mbps"],
        "throughput MB/s",
    )

    for bucket, results in data["snapshot"].items():
        for op in ("create", "list", "changes", "revert", "delete"):
            assert results[f"{op}_ok"], f"snapshot {bucket} {op} failed"
            _assert_lte(
                results[f"{op}_ms"],
                CAPSEM_BENCH_GATES["snapshot_op_ms"],
                f"snapshot {bucket} {op} latency",
            )

    if "storage" in data:
        validate_storage_split_result(data["storage"])


def validate_storage_split_result(data: dict[str, Any]) -> None:
    assert "kernel" in data, "storage kernel context missing"
    assert "cmdline" in data["kernel"], "storage kernel cmdline missing"
    assert "block_queues" in data["kernel"], "storage block queue metadata missing"
    assert "fuse_connections" in data["kernel"], "storage FUSE metadata missing"
    assert data["mounts"], "storage mountinfo is empty"
    assert "/" in data["paths"], "storage path metadata missing root path"
    assert "rootfs" in data, "storage rootfs section missing"
    assert "backing" in data["rootfs"], "storage rootfs backing metadata missing"
    kernel_args = set(data["kernel"].get("cmdline", {}).get("args", []))
    assert "capsem.rootfs=erofs" in kernel_args, (
        f"storage kernel cmdline must identify EROFS rootfs: {sorted(kernel_args)}"
    )
    backing = data["rootfs"]["backing"]
    assert backing.get("root_mount", {}).get("fs_type") == "overlay", (
        f"storage rootfs should run through overlay: {backing.get('root_mount')}"
    )
    assert backing.get("overlay_lowerdir"), (
        f"storage rootfs overlay lowerdir missing: {backing}"
    )
    squashfs = backing.get("squashfs_superblock", {})
    assert squashfs.get("error") == "not squashfs", (
        f"storage rootfs should not report a SquashFS backing: {squashfs}"
    )
    assert data["rootfs"]["seq_reads"], "storage rootfs seq_reads is empty"
    for item in data["rootfs"]["seq_reads"]:
        _assert_gte(
            item["cold"]["throughput_mbps"],
            CAPSEM_BENCH_GATES["storage_min_mbps"],
            f"storage rootfs {item['label']} cold read",
        )
        _assert_gte(
            item["warm"]["throughput_mbps"],
            CAPSEM_BENCH_GATES["storage_min_mbps"],
            f"storage rootfs {item['label']} warm read",
        )
    assert "writable" in data, "storage writable section missing"
    assert data["writable"], "storage writable section is empty"
    for path, item in data["writable"].items():
        if "skipped" in item or "error" in item:
            continue
        assert "io_profile" in item, f"storage {path} I/O profile missing"
        profile = item["io_profile"]
        assert profile["sequential"], f"storage {path} sequential profile empty"
        assert profile["random"], f"storage {path} random profile empty"
        assert "read_4k" in profile["random"], f"storage {path} random read missing"
        assert "write_4k_sync" in profile["random"], (
            f"storage {path} random sync write missing"
        )
        for workload, stats in profile["random"].items():
            _assert_gte(
                stats["iops"],
                CAPSEM_BENCH_GATES["storage_min_iops"],
                f"storage {path} {workload} IOPS",
            )
            assert "latency_ms" in stats, f"storage {path} {workload} latency missing"


def _assert_gte(value: float, gate: float, label: str) -> None:
    assert value >= gate, f"{label} {value:.1f} below {gate:.1f} gate"


def _assert_lte(value: float, gate: float, label: str) -> None:
    assert value <= gate, f"{label} {value:.1f} exceeds {gate:.1f} gate"


def _disk_rand_iops_gate() -> float:
    gates = cast(dict[str, int], CAPSEM_BENCH_GATES["disk_rand_iops"])
    if sys.platform.startswith("linux"):
        return gates["linux"]
    return gates["default"]
