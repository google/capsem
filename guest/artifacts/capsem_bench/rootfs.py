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
ROOTFS_LOWER_MOUNT = "/run/capsem-lower"
ROOTFS_RAND_READ_COUNT = 5000
ROOTFS_SMALL_READ_COUNT = 5000
ROOTFS_METADATA_STAT_COUNT = 10000
ROOTFS_LARGE_FILE_MIN_SIZE = 16 * 1024 * 1024
ROOTFS_SMALL_JS_MAX_SIZE = 64 * 1024
SMALL_FILE_SUFFIXES = (
    ".js", ".mjs", ".cjs", ".json", ".map", ".node", ".wasm",
    ".ts", ".tsx", ".jsx",
)


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


def collect_rootfs_workload_files(
    directories,
    *,
    large_min_size=ROOTFS_LARGE_FILE_MIN_SIZE,
    small_js_max_size=ROOTFS_SMALL_JS_MAX_SIZE,
):
    """Collect rootfs files split by workload shape."""
    all_files = []
    large_binaries = []
    small_js_files = []
    for d in directories:
        if not os.path.isdir(d):
            continue
        for root, _dirs, fnames in os.walk(d):
            for fname in fnames:
                fpath = os.path.join(root, fname)
                try:
                    st = os.lstat(fpath)
                except OSError:
                    continue
                if not stat.S_ISREG(st.st_mode):
                    continue
                item = (fpath, st.st_size)
                all_files.append(item)
                if st.st_size >= large_min_size:
                    large_binaries.append(item)
                suffix = os.path.splitext(fname)[1].lower()
                if suffix in SMALL_FILE_SUFFIXES and st.st_size <= small_js_max_size:
                    small_js_files.append(item)
    return {
        "all_files": all_files,
        "large_binaries": large_binaries,
        "small_js_files": small_js_files,
        "files_found": len(all_files),
    }


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


def bench_large_binary_reads(files, count=3):
    """Sequentially read the largest rootfs binaries, cold then warm."""
    if not files:
        return {"count": 0, "error": "no large files found"}

    selected = sorted(files, key=lambda item: item[1], reverse=True)[:count]
    reads = []
    for path, size in selected:
        cold = bench_rootfs_seq_read(path, size)
        warm = _bench_seq_read_no_drop(path, size)
        reads.append({
            "path": path,
            "size_bytes": size,
            "cold": cold,
            "warm": warm,
        })
    cold_total = sum(item["size_bytes"] for item in reads)
    cold_duration_ms = sum(item["cold"]["duration_ms"] for item in reads)
    warm_duration_ms = sum(item["warm"]["duration_ms"] for item in reads)
    return {
        "count": len(reads),
        "files": reads,
        "bytes_read": cold_total,
        "cold_duration_ms": round(cold_duration_ms, 1),
        "warm_duration_ms": round(warm_duration_ms, 1),
        "cold_throughput_mbps": throughput_mbps(cold_total, cold_duration_ms / 1000),
        "warm_throughput_mbps": throughput_mbps(cold_total, warm_duration_ms / 1000),
    }


def _bench_seq_read_no_drop(filepath, file_size):
    fd = os.open(filepath, os.O_RDONLY)
    try:
        start = time.monotonic()
        while os.read(fd, BLOCK_1M):
            pass
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


def bench_small_file_reads(files, count=ROOTFS_SMALL_READ_COUNT):
    """Read whole small JS/package files to model CLI loader behavior."""
    if not files:
        return {"count": 0, "error": "no small JS/package files found"}

    targets = [random.choice(files) for _ in range(count)]
    drop_caches()

    fd_cache = {}
    bytes_read = 0
    try:
        start = time.monotonic()
        for fpath, _size in targets:
            fd = fd_cache.get(fpath)
            if fd is None:
                fd = os.open(fpath, os.O_RDONLY)
                fd_cache[fpath] = fd
            data = os.pread(fd, ROOTFS_SMALL_JS_MAX_SIZE, 0)
            bytes_read += len(data)
        elapsed = time.monotonic() - start
    finally:
        for fd in fd_cache.values():
            os.close(fd)

    return {
        "count": count,
        "files_sampled": len(fd_cache),
        "bytes_read": bytes_read,
        "duration_ms": round(elapsed * 1000, 1),
        "ops_per_sec": round(count / elapsed, 1) if elapsed > 0 else 0,
        "throughput_mbps": throughput_mbps(bytes_read, elapsed),
    }


def bench_metadata_stat_walk(directories, max_entries=ROOTFS_METADATA_STAT_COUNT):
    """Measure rootfs metadata throughput with lstat over many entries."""
    drop_caches()
    entries = 0
    files = 0
    dirs = 0
    symlinks = 0
    errors = 0

    start = time.monotonic()
    for d in directories:
        if not os.path.isdir(d):
            continue
        for root, dirnames, filenames in os.walk(d):
            for name in dirnames + filenames:
                path = os.path.join(root, name)
                try:
                    st = os.lstat(path)
                except OSError:
                    errors += 1
                    continue
                entries += 1
                mode = st.st_mode
                if stat.S_ISDIR(mode):
                    dirs += 1
                elif stat.S_ISREG(mode):
                    files += 1
                elif stat.S_ISLNK(mode):
                    symlinks += 1
                if entries >= max_entries:
                    elapsed = time.monotonic() - start
                    return _metadata_summary(entries, files, dirs, symlinks, errors, elapsed)
    elapsed = time.monotonic() - start
    return _metadata_summary(entries, files, dirs, symlinks, errors, elapsed)


def lower_rootfs_scan_dirs(
    directories=ROOTFS_SCAN_DIRS,
    lower_mount=ROOTFS_LOWER_MOUNT,
):
    """Map rootfs scan directories to the mounted lower read-only filesystem."""
    if not os.path.isdir(lower_mount):
        return []

    lower_dirs = []
    for directory in directories:
        if not directory.startswith("/"):
            continue
        lower_dir = os.path.join(lower_mount, directory.lstrip("/"))
        if os.path.isdir(lower_dir):
            lower_dirs.append(lower_dir)
    return lower_dirs


def _metadata_summary(entries, files, dirs, symlinks, errors, elapsed):
    return {
        "entries": entries,
        "files": files,
        "dirs": dirs,
        "symlinks": symlinks,
        "errors": errors,
        "duration_ms": round(elapsed * 1000, 1),
        "stats_per_sec": round(entries / elapsed, 1) if elapsed > 0 else 0,
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

    workload_files = collect_rootfs_workload_files(ROOTFS_SCAN_DIRS)
    files = [(path, size) for path, size in workload_files["all_files"] if size >= BLOCK_4K]
    results["files_found"] = workload_files["files_found"]

    stats = bench_rootfs_rand_read(files, ROOTFS_RAND_READ_COUNT)
    results["rand_read_4k"] = stats
    if "error" not in stats:
        table.add_row("Rand read (4K)", f"{stats['files_sampled']} files",
                       f"{stats['throughput_mbps']} MB/s",
                       f"{stats['iops']:.0f}",
                       f"{stats['duration_ms']} ms")
    else:
        table.add_row("Rand read (4K)", stats["error"], "-", "-", "-")

    large_stats = bench_large_binary_reads(workload_files["large_binaries"])
    results["large_binary_seq_read"] = large_stats
    if "error" not in large_stats:
        table.add_row(
            "Large bin cold",
            f"{large_stats['count']} files",
            f"{large_stats['cold_throughput_mbps']} MB/s",
            "-",
            f"{large_stats['cold_duration_ms']} ms",
        )
        table.add_row(
            "Large bin warm",
            f"{large_stats['count']} files",
            f"{large_stats['warm_throughput_mbps']} MB/s",
            "-",
            f"{large_stats['warm_duration_ms']} ms",
        )
    else:
        table.add_row("Large binaries", large_stats["error"], "-", "-", "-")

    small_stats = bench_small_file_reads(workload_files["small_js_files"])
    results["small_js_read"] = small_stats
    if "error" not in small_stats:
        table.add_row(
            "Small JS reads",
            f"{small_stats['files_sampled']} files",
            f"{small_stats['throughput_mbps']} MB/s",
            f"{small_stats['ops_per_sec']:.0f}",
            f"{small_stats['duration_ms']} ms",
        )
    else:
        table.add_row("Small JS reads", small_stats["error"], "-", "-", "-")

    metadata_stats = bench_metadata_stat_walk(ROOTFS_SCAN_DIRS)
    results["metadata_stat"] = metadata_stats
    table.add_row(
        "Metadata stat",
        f"{metadata_stats['entries']} entries",
        "-",
        f"{metadata_stats['stats_per_sec']:.0f}",
        f"{metadata_stats['duration_ms']} ms",
    )

    lower_scan_dirs = lower_rootfs_scan_dirs()
    results["lower_scan_dirs"] = lower_scan_dirs
    if lower_scan_dirs:
        lower_metadata_stats = bench_metadata_stat_walk(lower_scan_dirs)
        results["metadata_stat_lower"] = lower_metadata_stats
        table.add_row(
            "Metadata lower",
            f"{lower_metadata_stats['entries']} entries",
            "-",
            f"{lower_metadata_stats['stats_per_sec']:.0f}",
            f"{lower_metadata_stats['duration_ms']} ms",
        )

    console.print(table)
    return results
