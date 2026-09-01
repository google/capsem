import ast
import copy
import importlib.util
import re
import tomllib
from pathlib import Path

import pytest
from helpers import benchmark_gates
from helpers.benchmark_gates import validate_capsem_bench_result

PROJECT_ROOT = Path(__file__).parent.parent


def _valid_result():
    return {
        "disk": {
            "seq_write": {"throughput_mbps": 500},
            "seq_read": {"throughput_mbps": 500},
            "rand_write_4k": {"iops": 5000},
            "rand_read_4k": {"iops": 5000},
        },
        "rootfs": {
            "seq_read": {"throughput_mbps": 300},
            "rand_read_4k": {"iops": 4000},
        },
        "startup": {
            "commands": {
                "python3": {"mean_ms": 10},
                "node": {"mean_ms": 150},
                "claude": {"mean_ms": 400},
                "gemini": {"mean_ms": 900},
                "codex": {"mean_ms": 350},
            },
        },
        "http": {
            "total_requests": 50,
            "successful": 50,
            "failed": 0,
            "requests_per_sec": 20,
            "latency_ms": {"p99": 300},
        },
        "throughput": {
            "http_code": 200,
            "size_bytes": 9_000_000,
            "throughput_mbps": 10,
        },
        "snapshot": {
            "10_files": {
                "create_ok": True,
                "list_ok": True,
                "changes_ok": True,
                "revert_ok": True,
                "delete_ok": True,
                "create_ms": 500,
                "list_ms": 300,
                "changes_ms": 300,
                "revert_ms": 300,
                "delete_ms": 300,
            },
            "100_files": {
                "create_ok": True,
                "list_ok": True,
                "changes_ok": True,
                "revert_ok": True,
                "delete_ok": True,
                "create_ms": 600,
                "list_ms": 300,
                "changes_ms": 300,
                "revert_ms": 300,
                "delete_ms": 300,
            },
            "500_files": {
                "create_ok": True,
                "list_ok": True,
                "changes_ok": True,
                "revert_ok": True,
                "delete_ok": True,
                "create_ms": 700,
                "list_ms": 300,
                "changes_ms": 300,
                "revert_ms": 300,
                "delete_ms": 300,
            },
        },
        "storage": {
            "kernel": {
                "cmdline": {
                    "raw": "capsem.rootfs=erofs ro",
                    "args": ["capsem.rootfs=erofs", "ro"],
                },
                "block_queues": {"vda": {"read_ahead_kb": 4096}},
                "fuse_connections": {},
                "known_host_queue_sizes": {
                    "kvm_virtio_blk": 256,
                    "kvm_virtio_fs": [256, 256],
                },
            },
            "mounts": [
                {
                    "mount_point": "/",
                    "fs_type": "ext4",
                    "source": "/dev/root",
                }
            ],
            "paths": {
                "/": {"exists": True, "writable": False},
                "/root": {"exists": True, "writable": True},
            },
            "rootfs": {
                "backing": {
                    "root_mount": {"fs_type": "overlay"},
                    "overlay_lowerdir": "/mnt/a",
                    "squashfs_superblock": {"error": "not squashfs"},
                },
                "seq_reads": [
                    {
                        "label": "largest",
                        "cold": {"throughput_mbps": 100},
                        "warm": {"throughput_mbps": 200},
                    }
                ],
                "rand_read_4k": {"iops": 1000},
            },
            "writable": {
                "/root": {
                    "seq_write": {"throughput_mbps": 100},
                    "seq_read_cold": {"throughput_mbps": 100},
                    "seq_read_warm": {"throughput_mbps": 200},
                    "rand_write_4k": {"iops": 1000},
                    "rand_read_4k": {"iops": 1000},
                    "io_profile": {
                        "sequential": {
                            "4k": {
                                "write": {
                                    "iops": 1000,
                                    "throughput_mbps": 4,
                                    "avg_latency_ms": 1,
                                },
                                "read_cold": {
                                    "iops": 1000,
                                    "throughput_mbps": 4,
                                    "avg_latency_ms": 1,
                                },
                                "read_warm": {
                                    "iops": 1000,
                                    "throughput_mbps": 4,
                                    "avg_latency_ms": 1,
                                },
                            }
                        },
                        "random": {
                            "read_4k": {
                                "iops": 1000,
                                "throughput_mbps": 4,
                                "avg_latency_ms": 1,
                                "latency_ms": {"p95": 1},
                            },
                            "write_4k_sync": {
                                "iops": 1000,
                                "throughput_mbps": 4,
                                "avg_latency_ms": 1,
                                "latency_ms": {"p95": 1},
                            },
                        },
                    },
                }
            },
        },
    }


def test_validate_capsem_bench_result_accepts_healthy_result():
    validate_capsem_bench_result(_valid_result())


def test_validate_capsem_bench_result_accepts_linux_virtiofs_sync_write_floor(
    monkeypatch,
):
    data = _valid_result()
    data["disk"]["rand_write_4k"]["iops"] = 425
    monkeypatch.setattr(benchmark_gates.sys, "platform", "linux")

    validate_capsem_bench_result(data)


def test_validate_capsem_bench_result_rejects_linux_virtiofs_sync_write_regression(
    monkeypatch,
):
    data = _valid_result()
    data["disk"]["rand_write_4k"]["iops"] = 350
    monkeypatch.setattr(benchmark_gates.sys, "platform", "linux")

    with pytest.raises(AssertionError, match="disk rand_write_4k"):
        validate_capsem_bench_result(data)


def test_release_protocol_benchmark_uses_release_scale():
    spec = importlib.util.spec_from_file_location(
        "test_capsem_bench_baseline",
        PROJECT_ROOT / "tests" / "capsem-serial" / "test_capsem_bench_baseline.py",
    )
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)

    assert module.RELEASE_PROTOCOL_REQUESTS >= 50_000
    assert module.RELEASE_PROTOCOL_CONCURRENCY == 64


def test_failed_capsem_bench_measurement_is_archived(monkeypatch):
    spec = importlib.util.spec_from_file_location(
        "test_capsem_bench_baseline",
        PROJECT_ROOT / "tests" / "capsem-serial" / "test_capsem_bench_baseline.py",
    )
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    data = _valid_result()
    data["snapshot"]["100_files"]["changes_ms"] = 10_000
    archived = []
    monkeypatch.setattr(
        module, "_save", lambda result: archived.append(copy.deepcopy(result))
    )

    with pytest.raises(AssertionError, match="snapshot 100_files changes"):
        module._archive_and_validate(data, "http://127.0.0.1:1234")

    assert archived == [data]
    assert data["mock_server_base_url"] == "http://127.0.0.1:1234"
    assert data["arch"]
    assert data["host_recorded_at"] > 0


@pytest.mark.parametrize(
    ("path", "value", "message"),
    [
        (("disk", "seq_write", "throughput_mbps"), 10, "disk seq_write"),
        (("startup", "commands", "gemini", "mean_ms"), 10_000, "startup gemini"),
        (("http", "failed"), 1, "HTTP failed"),
        (("throughput", "http_code"), 500, "throughput HTTP"),
        (("snapshot", "500_files", "changes_ok"), False, "snapshot 500_files changes"),
        (("snapshot", "100_files", "create_ms"), 10_000, "snapshot 100_files create"),
    ],
)
def test_validate_capsem_bench_result_rejects_bad_result(path, value, message):
    data = copy.deepcopy(_valid_result())
    target = data
    for key in path[:-1]:
        target = target[key]
    target[path[-1]] = value

    with pytest.raises(AssertionError, match=message):
        validate_capsem_bench_result(data)


def test_no_gross_regression_threshold_is_authored_in_python() -> None:
    """The gate contract, applied to the one table that escaped it.

    Eleven thresholds -- disk MB/s, IOPS, five per-runtime startup ceilings,
    HTTP rps and p99, throughput bytes and MB/s, snapshot op latency -- lived
    as literals in a test helper. Every other number the gate judges by lives
    in `config/gate.toml`, and these were judged the same way while being
    editable only by someone who knew the helper existed.
    """
    helper = PROJECT_ROOT / "tests" / "helpers" / "benchmark_gates.py"
    tree = ast.parse(helper.read_text(encoding="utf-8"))
    for node in ast.walk(tree):
        # An index is structure, not a threshold: `parents[2]` says where the
        # checkout root is.
        if isinstance(node, ast.Subscript):
            for inner in ast.walk(node.slice):
                inner.__dict__["_is_index"] = True
    authored = [
        node.value
        for node in ast.walk(tree)
        if isinstance(node, ast.Constant)
        and isinstance(node.value, (int, float))
        and not isinstance(node.value, bool)
        and not node.__dict__.get("_is_index")
        # `== 0` and `== 200` are correctness assertions -- no failed
        # requests, an HTTP 200 -- rather than performance thresholds.
        and node.value not in (0, 200)
    ]
    assert not authored, (
        "these thresholds are authored in the helper rather than read from "
        f"[benchmark.gates] in config/gate.toml: {authored}"
    )


def test_every_threshold_the_helper_uses_is_declared() -> None:
    """A key the helper reads but config does not declare fails at import.

    Which is the point: a threshold cannot go missing quietly and leave an
    assertion comparing against `None`.
    """
    config = tomllib.loads(
        (PROJECT_ROOT / "config" / "gate.toml").read_text(encoding="utf-8")
    )["benchmark"]["gates"]
    source = (PROJECT_ROOT / "tests" / "helpers" / "benchmark_gates.py").read_text(
        encoding="utf-8"
    )
    for key in re.findall(r'CAPSEM_BENCH_GATES\["(\w+)"\]', source):
        assert key in config, f"[benchmark.gates] does not declare {key!r}"
