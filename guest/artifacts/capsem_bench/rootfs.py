"""Rootfs read-only I/O benchmarks (squashfs where binaries live)."""

import os
import random
import stat
import time

from rich.table import Table

from .helpers import (
    BLOCK_1M, BLOCK_4K,
    console, drop_caches, fmt_bytes, throughput_mbps,
)

ROOTFS_SCAN_DIRS = ["/usr/bin", "/usr/lib", "/opt/ai-clis"]
ROOTFS_RAND_READ_COUNT = 5000


def find_largest_file(directories):
    """Find the largest regular file across the given directories."""
    best_path = None
    best_size = 0
    for d in directories:
        if not os.path.isdir(d):
            continue
        for root, _dirs, files in os.walk(d):
            for fname in files:
                fpath = os.path.join(root, fname)
                try:
                    st = os.lstat(fpath)
                    if not stat.S_ISREG(st.st_mode):
                        continue
                    if st.st_size > best_size:
                        best_size = st.st_size
                        best_path = fpath
                except OSError:
                    continue
    return best_path, best_size


def collect_rootfs_files(directories, min_size=BLOCK_4K):
    """Collect regular files from rootfs directories for random read testing."""
    files = []
    for d in directories:
        if not os.path.isdir(d):
            continue
        for root, _dirs, fnames in os.walk(d):
            for fname in fnames:
                fpath = os.path.join(root, fname)
                try:
                    st = os.lstat(fpath)
                    if stat.S_ISREG(st.st_mode) and st.st_size >= min_size:
                        files.append((fpath, st.st_size))
                except OSError:
                    continue
    return files


def bench_rootfs_seq_read(filepath, file_size):
    """Sequential read of a rootfs file with 1MB blocks after drop_caches."""
    drop_caches()

    fd = os.open(filepath, os.O_RDONLY)
    try:
        start = time.monotonic()
        while True:
            data = os.read(fd, BLOCK_1M)
            if not data:
                break
        elapsed = time.monotonic() - start
    finally:
        os.close(fd)

    return {
        "file": filepath,
        "size_bytes": file_size,
        "block_size": BLOCK_1M,
        "duration_ms": round(elapsed * 1000, 1),
        "throughput_mbps": throughput_mbps(file_size, elapsed),
    }


def bench_rootfs_rand_read(files, count):
    """Random 4K reads across multiple rootfs files after drop_caches."""
    if not files:
        return {"count": 0, "error": "no files found"}

    targets = []
    for _ in range(count):
        fpath, fsize = random.choice(files)
        max_off = fsize - BLOCK_4K
        if max_off < 0:
            max_off = 0
        offset = random.randint(0, max_off) & ~(BLOCK_4K - 1)
        targets.append((fpath, offset))

    drop_caches()

    fd_cache = {}
    try:
        start = time.monotonic()
        for fpath, offset in targets:
            if fpath not in fd_cache:
                fd_cache[fpath] = os.open(fpath, os.O_RDONLY)
            os.pread(fd_cache[fpath], BLOCK_4K, offset)
        elapsed = time.monotonic() - start
    finally:
        for fd in fd_cache.values():
            os.close(fd)

    total_bytes = count * BLOCK_4K
    iops = round(count / elapsed, 1) if elapsed > 0 else 0
    return {
        "count": count,
        "files_sampled": len(fd_cache),
        "block_size": BLOCK_4K,
        "duration_ms": round(elapsed * 1000, 1),
        "iops": iops,
        "throughput_mbps": throughput_mbps(total_bytes, elapsed),
    }


def rootfs_bench():
    """Run rootfs read-only I/O benchmarks."""
    table = Table(title="Rootfs Read I/O")
    table.add_column("Test", style="bold")
    table.add_column("Detail")
    table.add_column("Throughput", justify="right")
    table.add_column("IOPS", justify="right")
    table.add_column("Duration", justify="right")

    results = {"scan_dirs": ROOTFS_SCAN_DIRS}

    largest_path, largest_size = find_largest_file(ROOTFS_SCAN_DIRS)
    if largest_path:
        results["largest_file"] = largest_path
        results["largest_file_size"] = largest_size

        stats = bench_rootfs_seq_read(largest_path, largest_size)
        results["seq_read"] = stats
        table.add_row("Seq read (1MB)", f"{os.path.basename(largest_path)} ({fmt_bytes(largest_size)})",
                       f"{stats['throughput_mbps']} MB/s", "-",
                       f"{stats['duration_ms']} ms")
    else:
        results["seq_read"] = {"error": "no files found in scan dirs"}
        table.add_row("Seq read (1MB)", "no files found", "-", "-", "-")

    files = collect_rootfs_files(ROOTFS_SCAN_DIRS)
    results["files_found"] = len(files)

    stats = bench_rootfs_rand_read(files, ROOTFS_RAND_READ_COUNT)
    results["rand_read_4k"] = stats
    if "error" not in stats:
        table.add_row("Rand read (4K)", f"{stats['files_sampled']} files",
                       f"{stats['throughput_mbps']} MB/s",
                       f"{stats['iops']:.0f}",
                       f"{stats['duration_ms']} ms")
    else:
        table.add_row("Rand read (4K)", stats["error"], "-", "-", "-")

    console.print(table)
    return results
