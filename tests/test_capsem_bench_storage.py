import sys
import struct
from pathlib import Path

PROJECT_ROOT = Path(__file__).parent.parent
sys.path.insert(0, str(PROJECT_ROOT / "guest" / "artifacts"))

from capsem_bench.storage import (  # noqa: E402
    find_mount_for_path,
    io_profile_bench,
    kernel_storage_context,
    read_block_queues,
    read_fuse_connections,
    read_kernel_cmdline,
    parse_mount_options,
    parse_mountinfo,
    parse_squashfs_superblock,
    path_stat,
    rootfs_backing_metadata,
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


def test_parse_mount_options_splits_key_value_options():
    options = parse_mount_options("rw,lowerdir=/mnt/a,upperdir=/mnt/system/upper")

    assert options["rw"] is True
    assert options["lowerdir"] == "/mnt/a"
    assert options["upperdir"] == "/mnt/system/upper"


def test_parse_squashfs_superblock_reports_chunk_and_compression():
    data = bytearray(96)
    data[:32] = struct.pack(
        "<IIIIIHHHHHH",
        0x73717368,
        1234,
        1_700_000_000,
        65_536,
        42,
        6,
        16,
        0,
        1,
        4,
        0,
    )

    info = parse_squashfs_superblock(bytes(data), device="/dev/vda")

    assert info["compression"] == "zstd"
    assert info["block_size_bytes"] == 65_536
    assert info["block_size"] == "64.0 KB"
    assert info["version"] == "4.0"


def test_read_kernel_cmdline_splits_arguments(tmp_path):
    cmdline = tmp_path / "cmdline"
    cmdline.write_text("root=/dev/vda ro capsem.storage=virtiofs\n")

    info = read_kernel_cmdline(str(cmdline))

    assert info["raw"] == "root=/dev/vda ro capsem.storage=virtiofs"
    assert "capsem.storage=virtiofs" in info["args"]


def test_read_block_queues_reports_scheduler_and_depth(tmp_path):
    queue = tmp_path / "vda" / "queue"
    queue.mkdir(parents=True)
    (queue / "scheduler").write_text("mq-deadline [none]\n")
    (queue / "read_ahead_kb").write_text("4096\n")
    (queue / "nr_requests").write_text("256\n")

    info = read_block_queues(str(tmp_path))

    assert info["vda"]["selected_scheduler"] == "none"
    assert info["vda"]["read_ahead_kb"] == 4096
    assert info["vda"]["nr_requests"] == 256


def test_read_fuse_connections_reports_backpressure_knobs(tmp_path):
    conn = tmp_path / "7"
    conn.mkdir()
    (conn / "max_background").write_text("12\n")
    (conn / "congestion_threshold").write_text("9\n")
    (conn / "waiting").write_text("0\n")

    info = read_fuse_connections(str(tmp_path))

    assert info["7"]["max_background"] == 12
    assert info["7"]["congestion_threshold"] == 9
    assert info["7"]["waiting"] == 0


def test_kernel_storage_context_includes_known_host_queue_sizes(monkeypatch):
    monkeypatch.setattr(
        "capsem_bench.storage.read_kernel_cmdline",
        lambda: {"raw": "root=/dev/vda", "args": ["root=/dev/vda"]},
    )
    monkeypatch.setattr("capsem_bench.storage.read_block_queues", lambda: {})
    monkeypatch.setattr("capsem_bench.storage.read_fuse_connections", lambda: {})

    info = kernel_storage_context()

    assert info["known_host_queue_sizes"]["kvm_virtio_blk"] == 256
    assert info["known_host_queue_sizes"]["kvm_virtio_fs"] == [256, 256]


def test_find_mount_for_path_uses_longest_prefix():
    mounts = [
        {"mount_point": "/", "fs_type": "ext4"},
        {"mount_point": "/root", "fs_type": "virtiofs"},
        {"mount_point": "/root/project", "fs_type": "tmpfs"},
    ]

    assert find_mount_for_path("/root/project/file.txt", mounts)["fs_type"] == "tmpfs"
    assert find_mount_for_path("/root/other.txt", mounts)["fs_type"] == "virtiofs"
    assert find_mount_for_path("/usr/bin/python3", mounts)["fs_type"] == "ext4"


def test_rootfs_backing_metadata_includes_overlay_and_superblock(monkeypatch):
    mounts = [
        {
            "mount_point": "/",
            "fs_type": "overlay",
            "source": "overlay",
            "options": "rw,lowerdir=/mnt/a,upperdir=/mnt/system/upper,workdir=/mnt/system/work",
        },
        {
            "mount_point": "/mnt/a",
            "fs_type": "squashfs",
            "source": "/dev/vda",
            "options": "ro",
        },
    ]

    monkeypatch.setattr(
        "capsem_bench.storage.read_squashfs_superblock",
        lambda device: {"device": device, "compression": "zstd", "block_size_bytes": 65_536},
    )

    info = rootfs_backing_metadata(mounts)

    assert info["overlay_lowerdir"] == "/mnt/a"
    assert info["overlay_upperdir"] == "/mnt/system/upper"
    assert info["squashfs_mounts"][0]["source"] == "/dev/vda"
    assert info["squashfs_superblock"]["block_size_bytes"] == 65_536


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
