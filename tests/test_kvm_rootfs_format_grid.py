import json
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "kvm_rootfs_format_grid.py"


def test_rootfs_format_grid_dry_run_crosses_formats_and_shapes():
    proc = subprocess.run(
        [
            sys.executable,
            str(SCRIPT),
            "--dry-run",
            "--formats",
            "squashfs-zstd,squashfs-uncompressed",
            "--queue-counts",
            "1,8",
            "--queue-sizes",
            "128",
            "--seg-maxes",
            "64",
            "--logical-block-sizes",
            "4096",
        ],
        cwd=ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=True,
    )

    payload = json.loads(proc.stdout)
    assert payload["formats"] == ["squashfs-zstd", "squashfs-uncompressed"]
    assert payload["count"] == 4
    assert payload["shapes"] == [
        {
            "queue_count": 1,
            "queue_size": 128,
            "seg_max": 64,
            "logical_block_size": 4096,
        },
        {
            "queue_count": 8,
            "queue_size": 128,
            "seg_max": 64,
            "logical_block_size": 4096,
        },
    ]


def test_rootfs_format_grid_rejects_unknown_format():
    proc = subprocess.run(
        [
            sys.executable,
            str(SCRIPT),
            "--dry-run",
            "--formats",
            "squashfs-zstd,unknownfs",
        ],
        cwd=ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )

    assert proc.returncode != 0
    assert "unknown rootfs format" in proc.stderr
