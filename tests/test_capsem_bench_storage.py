import sys
from pathlib import Path

PROJECT_ROOT = Path(__file__).parent.parent
sys.path.insert(0, str(PROJECT_ROOT / "guest" / "artifacts"))

from capsem_bench.storage import (  # noqa: E402
    find_mount_for_path,
    io_profile_bench,
    parse_mountinfo,
    path_stat,
    storage_paths,
)


def test_parse_mountinfo_extracts_mount_points():
    text = (
        "27 23 0:24 / / rw,relatime - ext4 /dev/root rw\n"
        "28 27 0:25 /workspace /root rw,relatime - virtiofs capsem rw\n"
    )

    mounts = parse_mountinfo(text)

    assert mounts[0]["mount_point"] == "/"
    assert mounts[0]["fs_type"] == "ext4"
    assert mounts[1]["mount_point"] == "/root"
    assert mounts[1]["source"] == "capsem"


def test_find_mount_for_path_uses_longest_prefix():
    mounts = [
        {"mount_point": "/", "fs_type": "ext4"},
        {"mount_point": "/root", "fs_type": "virtiofs"},
        {"mount_point": "/root/project", "fs_type": "tmpfs"},
    ]

    assert find_mount_for_path("/root/project/file.txt", mounts)["fs_type"] == "tmpfs"
    assert find_mount_for_path("/root/other.txt", mounts)["fs_type"] == "virtiofs"
    assert find_mount_for_path("/usr/bin/python3", mounts)["fs_type"] == "ext4"


def test_path_stat_reports_existing_path(tmp_path):
    info = path_stat(str(tmp_path), [])

    assert info["exists"] is True
    assert info["path"] == str(tmp_path)
    assert info["writable"] is True
    assert info["statvfs"]["block_size"] > 0


def test_storage_paths_are_deduped(monkeypatch):
    monkeypatch.setenv("CAPSEM_STORAGE_BENCH_PATHS", "/root:/root:/tmp")

    assert storage_paths() == ["/root", "/tmp"]


def test_io_profile_reports_sequential_and_random_iops(tmp_path):
    profile = io_profile_bench(
        str(tmp_path),
        size_mb=1,
        seq_block_sizes=(4096,),
        rand_op_count=8,
    )

    assert profile["size_mb"] == 1
    assert profile["random_ops"] == 8
    assert profile["sequential"]["4k"]["write"]["iops"] > 0
    assert profile["sequential"]["4k"]["read_cold"]["throughput_mbps"] > 0
    assert profile["sequential"]["4k"]["read_warm"]["avg_latency_ms"] >= 0
    assert profile["random"]["read_4k"]["iops"] > 0
    assert profile["random"]["read_4k"]["latency_ms"]["p95"] >= 0
    assert profile["random"]["write_4k_sync"]["sync_each"] is True
    assert profile["random"]["write_4k_sync"]["latency_ms"]["p95"] >= 0
