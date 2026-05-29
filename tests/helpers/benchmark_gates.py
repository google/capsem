"""Gross-regression gates for benchmark JSON artifacts."""

from __future__ import annotations

from typing import Any


CAPSEM_BENCH_GATES = {
    "disk_seq_mbps": 50,
    "disk_rand_iops": 1_000,
    "rootfs_seq_mbps": 100,
    "rootfs_rand_iops": 1_000,
    "startup_mean_ms": {
        "python3": 100,
        "node": 750,
        "claude": 2_500,
        "gemini": 5_000,
        "codex": 2_500,
    },
    "http_min_rps": 5,
    "http_p99_ms": 5_000,
    "throughput_min_bytes": 1_000_000,
    "throughput_min_mbps": 1,
    "snapshot_op_ms": 5_000,
}


def validate_capsem_bench_result(data: dict[str, Any]) -> None:
    disk = data["disk"]
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
        CAPSEM_BENCH_GATES["disk_rand_iops"],
        "disk rand_write_4k IOPS",
    )
    _assert_gte(
        disk["rand_read_4k"]["iops"],
        CAPSEM_BENCH_GATES["disk_rand_iops"],
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


def _assert_gte(value: float, gate: float, label: str) -> None:
    assert value >= gate, f"{label} {value:.1f} below {gate:.1f} gate"


def _assert_lte(value: float, gate: float, label: str) -> None:
    assert value <= gate, f"{label} {value:.1f} exceeds {gate:.1f} gate"
