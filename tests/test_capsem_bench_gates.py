import copy

import pytest

from helpers.benchmark_gates import validate_capsem_bench_result


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
                "cmdline": {"raw": "root=/dev/vda ro", "args": ["root=/dev/vda", "ro"]},
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
                    "squashfs_superblock": {
                        "compression": "zstd",
                        "block_size_bytes": 65_536,
                    },
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
