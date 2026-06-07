import os
import sys
from pathlib import Path

PROJECT_ROOT = Path(__file__).parent.parent
sys.path.insert(0, str(PROJECT_ROOT / "guest" / "artifacts"))

from capsem_bench import rootfs


def _write(path, data=b"x"):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)


def test_collect_rootfs_workload_files_splits_large_binaries_and_small_js(tmp_path):
    _write(tmp_path / "bin" / "tool", b"x" * 128)
    _write(tmp_path / "opt" / "pkg" / "index.js", b"console.log(1)")
    _write(tmp_path / "opt" / "pkg" / "nested" / "config.json", b"{}")
    _write(tmp_path / "opt" / "pkg" / "huge.js", b"x" * 512)
    _write(tmp_path / "usr" / "lib" / "libstuff.so", b"y" * 256)

    profile = rootfs.collect_rootfs_workload_files(
        [str(tmp_path)],
        large_min_size=128,
        small_js_max_size=64,
    )

    large_names = {os.path.basename(path) for path, _size in profile["large_binaries"]}
    small_names = {os.path.basename(path) for path, _size in profile["small_js_files"]}

    assert large_names == {"tool", "huge.js", "libstuff.so"}
    assert small_names == {"index.js", "config.json"}
    assert profile["files_found"] == 5


def test_metadata_stat_walk_counts_entries_and_reports_rate(tmp_path, monkeypatch):
    monkeypatch.setattr(rootfs, "drop_caches", lambda: None)
    for idx in range(10):
        _write(tmp_path / "node_modules" / f"pkg{idx}" / "index.js", b"x")

    stats = rootfs.bench_metadata_stat_walk([str(tmp_path)], max_entries=30)

    assert stats["entries"] == 21
    assert stats["dirs"] >= 1
    assert stats["files"] >= 1
    assert stats["errors"] == 0
    assert stats["stats_per_sec"] > 0


def test_small_file_reads_reports_ops_and_bytes(tmp_path, monkeypatch):
    monkeypatch.setattr(rootfs, "drop_caches", lambda: None)
    files = []
    for idx in range(4):
        path = tmp_path / f"small{idx}.js"
        _write(path, b"x" * (idx + 1))
        files.append((str(path), idx + 1))

    stats = rootfs.bench_small_file_reads(files, count=8)

    assert stats["count"] == 8
    assert stats["files_sampled"] <= 4
    assert stats["bytes_read"] >= 8
    assert stats["ops_per_sec"] > 0
